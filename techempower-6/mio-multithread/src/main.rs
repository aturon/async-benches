#[macro_use]
extern crate log;
extern crate mio_multithread;
extern crate env_logger;
extern crate httparse;
extern crate mio;
extern crate num_cpus;
extern crate slab;
extern crate time;

use std::ascii::AsciiExt;
use std::cell::UnsafeCell;
use std::env;
use std::fmt;
use std::io::{self, Read, Write};
use std::mem;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::slice;
use std::str;
use std::sync::Arc;
use std::sync::atomic::Ordering::{SeqCst, Acquire, Release};
use std::sync::atomic::{AtomicUsize, AtomicBool};
use std::thread;

use mio::tcp::{TcpStream, TcpListener};
use mio::{Poll, Token, EventSet, PollOpt, Events};

const LISTENER: mio::Token = mio::Token(0);

struct Server {
    listener: TcpListener,
    connections: Vec<Slot>,
    next: AtomicUsize,
}

struct Slot {
    state: AtomicUsize,
    events: AtomicUsize,
    lock: Lock<Option<Connection>>,
}

impl Server {
    fn new(listener: TcpListener) -> Server {
        Server {
            listener: listener,
            connections: (0..1024).map(|i| {
                let next = if i == 1023 {
                    usize::max_value()
                } else {
                    i + 1
                };
                Slot {
                    state: AtomicUsize::new(next),
                    lock: Lock::new(None),
                    events: AtomicUsize::new(0),
                }
            }).collect(),
            next: AtomicUsize::new(0),
        }
    }

    fn ready(&self,
             idx: usize,
             poll: &Poll,
             token: Token,
             events: EventSet) {
        debug!("srv[{}] - {:?} {:?}", idx, token, events);
        if token == LISTENER {
            while let Ok(Some(socket)) = self.listener.accept() {
                debug!("accepted");
                let token = self.insert(socket.0);
                self.try_connection(poll, token, EventSet::all());
            }
        } else {
            self.try_connection(poll, token, events);
        }
    }

    fn insert(&self, socket: TcpStream) -> mio::Token {
        // First, acquire the location in the vec we want.
        //
        // Load the next pointer, and then see if that slot is available. If
        // it's not we try again. If it's the end of the list we panic.
        // Otherwise we attempt to change the next pointer to where that slot is
        // pointing at.
        //
        // If that fails, we try everything again, if that succeeds then we've
        // now taken this slot.
        let mut idx = self.next.load(SeqCst);
        loop {
            debug!("attempting to acquire {}", idx);
            let next = self.connections[idx].state.load(SeqCst);
            if next == 0 {
                idx = self.next.load(SeqCst);
                continue
            }
            if next == usize::max_value() {
                panic!("vec is full")
            }
            match self.next.compare_exchange(idx, next, SeqCst, SeqCst) {
                Ok(_) => {
                    debug!("next is now {}", next);
                    break
                }
                Err(i) => idx = i,
            }
        }

        // Once we've got the slot, transition it to the "in use" state.
        let prev = self.connections[idx].state.swap(0, SeqCst);
        assert!(prev != 0);

        // Now put our connection into the slot
        let slot = self.connections[idx].lock.try_lock();
        let mut slot = slot.expect("failed lock");
        *slot = Some(Connection::new(socket));
        mio::Token(idx + 1)
    }

    fn try_connection(&self, poll: &Poll, token: Token, events: EventSet) {
        let idx = token.as_usize() - 1;

        // make sure slot is in use
        let slot = &self.connections[idx];
        assert_eq!(slot.state.load(SeqCst), 0);

        // WARNING: this is crazy
        //
        // Ask acrichto if you want to know what's happening before you try to
        // read yourself. This is probably wrong as well.

        let prev = slot.events.fetch_or((events.bits() << 1) | 1, SeqCst);
        if prev & 1 == 1 {
            return debug!("deferring for someone else");
        }

        let mut remove = false;
        loop {
            let bits = slot.events.swap(1, SeqCst);
            assert!(bits & 1 == 1);
            if bits == 1 {
                match slot.events.compare_exchange(1, 0, SeqCst, SeqCst) {
                    Ok(_) => break,
                    // TODO: optimize this
                    Err(_) => continue,
                }
            }
            let mut events = EventSet::none();
            if (bits >> 1) & EventSet::readable().bits() != 0 {
                events = events | EventSet::readable();
            }
            if (bits >> 1) & EventSet::writable().bits() != 0 {
                events = events | EventSet::writable();
            }
            let mut conn = slot.lock.try_lock().expect("contention");
            remove = remove || {
                let conn = conn.as_mut().expect("conn is none");

                if conn.first {
                    poll.register(&conn.socket,
                                  token,
                                  EventSet::readable() | EventSet::writable(),
                                  PollOpt::edge()).unwrap();
                    conn.first = false;
                }

                let res = conn.ready(events);

                if res.is_err() || conn.closed {
                    if let Err(ref e) = res {
                        info!("error: {:?}\non: {:?} {:?}", e, token, conn.socket);
                    }
                    debug!("removing");
                    true
                } else {
                    false
                }
            };
            if remove {
                *conn = None;
            }
        }
        if remove {
            self.remove(idx);
        }
    }

    fn remove(&self, idx: usize) {
        let slot = &self.connections[idx];
        assert_eq!(slot.events.swap(0, SeqCst), 0);
        let mut next = self.next.load(SeqCst);
        loop {
            slot.state.store(next, SeqCst);
            match self.next.compare_exchange(next, idx, SeqCst, SeqCst) {
                Ok(_) => break,
                Err(n) => next = n,
            }
        }
    }
}

struct Connection {
    socket: TcpStream,
    input: Vec<u8>,
    output: Output,
    keepalive: bool,
    closed: bool,
    events: EventSet,
    first: bool,
    read_closed: bool,
}

struct Output {
    buf: Vec<u8>,
}

impl Connection {
    fn new(socket: TcpStream) -> Connection {
        Connection {
            socket: socket,
            input: Vec::with_capacity(2048),
            output: Output {
                buf: Vec::with_capacity(2048),
            },
            keepalive: false,
            read_closed: false,
            closed: false,
            first: true,
            events: EventSet::none(),
        }
    }

    fn ready(&mut self, events: EventSet) -> io::Result<()> {
        self.events = self.events | events;
        while self.events.is_readable() && !self.read_closed {
            let before = self.input.len();
            let eof = try!(read(&mut self.socket,
                                &mut self.input,
                                &mut self.events));
            if eof {
                debug!("eof");
            }
            self.read_closed = eof;
            if self.input.len() == before {
                break
            }

            while self.input.len() > 0 {
                let (req, amt) = match try!(parse(&self.input)) {
                    Some(pair) => pair,
                    None => {
                        debug!("need more data for a request");
                        return Ok(())
                    }
                };
                let request = Request {
                    inner: req,
                    amt: amt,
                    data: mem::replace(&mut self.input, Vec::new()),
                };
                debug!("got a request");
                self.keepalive = request.version() >= 1;
                self.keepalive = self.keepalive || request.headers().any(|s| {
                    s.0.eq_ignore_ascii_case("connection") &&
                        s.1.eq_ignore_ascii_case(b"keep-alive")
                });

                let response = process(&request);
                response.to_bytes(&mut self.output.buf);

                self.input = request.into_input_buf();
                if !self.keepalive {
                    debug!("disabling keepalive");
                    self.read_closed = true;
                }
            }
        }

        if self.events.is_writable() && self.output.buf.len() > 0 {
            let done = try!(write(&mut self.socket,
                                  &mut self.output,
                                  &mut self.events));
            if done {
                debug!("wrote response");
                if !self.keepalive || self.read_closed {
                    self.closed = true;
                }
            }
        }

        if self.read_closed && self.output.buf.len() == 0 {
            self.closed = true;
        }
        Ok(())
    }
}

fn process(r: &Request) -> Response {
    assert!(r.path() == "/plaintext");
    let mut r = Response::new();
    r.header("Content-Type", "text/plain; charset=UTF-8")
     .body("Hello, World!");
    return r
}

type Slice = (usize, usize);

#[allow(dead_code)]
pub struct Request {
    inner: RawRequest,
    data: Vec<u8>,
    amt: usize,
}

struct RawRequest {
    method: Slice,
    path: Slice,
    version: u8,
    headers: Vec<(Slice, Slice)>,
}

pub struct Headers<'a> {
    iter: slice::Iter<'a, (Slice, Slice)>,
    req: &'a Request,
}

impl Request {
    pub fn method(&self) -> &str {
        str::from_utf8(self.get(&self.inner.method)).unwrap()
    }

    pub fn path(&self) -> &str {
        str::from_utf8(self.get(&self.inner.path)).unwrap()
    }

    pub fn version(&self) -> u8 {
        self.inner.version
    }

    pub fn headers(&self) -> Headers {
        Headers {
            iter: self.inner.headers.iter(),
            req: self,
        }
    }

    pub fn into_input_buf(mut self) -> Vec<u8> {
        self.data.drain(..self.amt);
        self.data
    }

    fn get(&self, s: &Slice) -> &[u8] {
        &self.data[s.0..s.1]
    }
}

impl<'a> Iterator for Headers<'a> {
    type Item = (&'a str, &'a [u8]);

    fn next(&mut self) -> Option<(&'a str, &'a [u8])> {
        self.iter.next().map(|&(ref k, ref v)| {
            (str::from_utf8(self.req.get(k)).unwrap(), self.req.get(v))
        })
    }
}

fn parse(buf: &[u8]) -> io::Result<Option<(RawRequest, usize)>> {
    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut r = httparse::Request::new(&mut headers);
    let status = try!(r.parse(&buf).map_err(|_| {
        io::Error::new(io::ErrorKind::Other, "failed to parse")
    }));
    return match status {
        httparse::Status::Complete(amt) => {
            debug!("ok {:?}", String::from_utf8_lossy(&buf[..amt]));
            Ok(Some((RawRequest {
                method: slice(buf, r.method.unwrap().as_bytes()),
                path: slice(buf, r.path.unwrap().as_bytes()),
                version: r.version.unwrap(),
                headers: r.headers.iter().map(|h| {
                    (slice(buf, h.name.as_bytes()), slice(buf, &h.value))
                }).collect(),
            }, amt)))
        }
        httparse::Status::Partial => Ok(None),
    };

    fn slice(buf: &[u8], inner: &[u8]) -> Slice {
        let start = inner.as_ptr() as usize - buf.as_ptr() as usize;
        assert!(start < buf.len());
        (start, start + inner.len())
    }
}


pub struct Response {
    headers: Vec<(String, String)>,
    response: String,
}

impl Response {
    pub fn new() -> Response {
        Response {
            headers: Vec::new(),
            response: String::new(),
        }
    }

    pub fn header(&mut self, name: &str, val: &str) -> &mut Response {
        self.headers.push((name.to_string(), val.to_string()));
        self
    }

    pub fn body(&mut self, s: &str) -> &mut Response {
        self.response = s.to_string();
        self
    }

    fn to_bytes(&self, into: &mut Vec<u8>) {
        use std::fmt::Write;

        write!(FastWrite(into), "\
            HTTP/1.1 200 OK\r\n\
            Server: Example\r\n\
            Date: {}\r\n\
            Content-Length: {}\r\n\
        ", mio_multithread::date::now(), self.response.len()).unwrap();
        for &(ref k, ref v) in &self.headers {
            extend(into, k.as_bytes());
            extend(into, b": ");
            extend(into, v.as_bytes());
            extend(into, b"\r\n");
        }
        extend(into, b"\r\n");
        extend(into, self.response.as_bytes());
    }
}

// TODO: impl fmt::Write for Vec<u8>
//
// Right now `write!` on `Vec<u8>` goes through io::Write and is not super
// speedy, so inline a less-crufty implementation here which doesn't go through
// io::Error.
struct FastWrite<'a>(&'a mut Vec<u8>);

impl<'a> fmt::Write for FastWrite<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        extend(self.0, s.as_bytes());
        Ok(())
    }
}

// TODO: why does extend_from_slice not optimize?
fn extend(dst: &mut Vec<u8>, data: &[u8]) {
    use std::ptr;
    dst.reserve(data.len());
    let prev = dst.len();
    unsafe {
        ptr::copy_nonoverlapping(data.as_ptr(),
                                 dst.as_mut_ptr().offset(prev as isize),
                                 data.len());
        dst.set_len(prev + data.len());
    }
}

fn read(socket: &mut TcpStream,
        input: &mut Vec<u8>,
        events: &mut EventSet) -> io::Result<bool> {
    match socket.read(unsafe { slice_to_end(input) }) {
        Ok(0) => return Ok(true),
        Ok(n) => {
            let len = input.len();
            unsafe { input.set_len(len + n); }
            return Ok(false)
        }
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
            *events = *events & !EventSet::readable();
            return Ok(false)
        }
        Err(e) => return Err(e),
    }

    unsafe fn slice_to_end(v: &mut Vec<u8>) -> &mut [u8] {
        use std::slice;
        if v.capacity() == 0 {
            v.reserve(16);
        }
        if v.capacity() == v.len() {
            v.reserve(1);
        }
        slice::from_raw_parts_mut(v.as_mut_ptr().offset(v.len() as isize),
                                  v.capacity() - v.len())
    }
}

fn write(socket: &mut TcpStream,
         output: &mut Output,
         events: &mut EventSet) -> io::Result<bool> {
    assert!(output.buf.len() > 0);
    loop {
        match socket.write(&output.buf) {
            Ok(0) => {
                return Err(io::Error::new(io::ErrorKind::Other, "early eof2"))
            }
            Ok(n) => {
                output.buf.drain(..n);
                if output.buf.len() == 0 {
                    return Ok(true)
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                *events = *events & !EventSet::writable();
                return Ok(false)
            }
            Err(e) => return Err(e),
        }
    }
}


fn main() {
    env_logger::init().unwrap();

    let poll = Arc::new(mio::Poll::new().unwrap());
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();
    let listener = TcpListener::bind(&addr).unwrap();
    poll.register(&listener,
                  LISTENER,
                  EventSet::readable(),
                  PollOpt::level()).unwrap();
    let server = Arc::new(Server::new(listener));

    let threads = (0..8).map(|n| {
        let poll = poll.clone();
        let server = server.clone();
        thread::spawn(move || {
            let mut events = Events::new();
            loop {
                poll.poll(&mut events, None).unwrap();

                for i in 0..events.len() {
                    let event = events.get(i).unwrap();
                    server.ready(n, &poll, event.token(), event.kind());
                }
            }
        })
    }).collect::<Vec<_>>();

    for thread in threads {
        thread.join().unwrap();
    }
}

// Utilities from elsewhere

pub struct Lock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

pub struct TryLock<'a, T: 'a> {
    __ptr: &'a Lock<T>,
}

unsafe impl<T: Send> Send for Lock<T> {}
unsafe impl<T: Send> Sync for Lock<T> {}

impl<T> Lock<T> {
    pub fn new(t: T) -> Lock<T> {
        Lock {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(t),
        }
    }

    pub fn try_lock(&self) -> Option<TryLock<T>> {
        if !self.locked.swap(true, Acquire) {
            Some(TryLock { __ptr: self })
        } else {
            None
        }
    }
}

impl<'a, T> Deref for TryLock<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.__ptr.data.get() }
    }
}

impl<'a, T> DerefMut for TryLock<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.__ptr.data.get() }
    }
}

impl<'a, T> Drop for TryLock<'a, T> {
    fn drop(&mut self) {
        self.__ptr.locked.store(false, Release);
    }
}
