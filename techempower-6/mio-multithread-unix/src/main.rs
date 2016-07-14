#[macro_use]
extern crate log;
extern crate env_logger;
extern crate httparse;
extern crate mio;
extern crate net2;
extern crate num_cpus;
extern crate slab;
extern crate time;

use std::ascii::AsciiExt;
use std::env;
use std::io::{self, Read, Write};
use std::mem;
use std::net::SocketAddr;
use std::slice;
use std::str;
use std::thread;

use net2::unix::*;

use mio::tcp::{TcpStream, TcpListener};
use mio::{Poll, Token, EventSet, PollOpt, Events};
use slab::Slab;

const LISTENER: mio::Token = mio::Token(0);

struct Server<'a> {
    count: u64,
    listener: &'a TcpListener,
    connections: Slab<Connection, mio::Token>,
}

impl<'a> Server<'a> {
    fn new(listener: &'a TcpListener) -> Server<'a> {
        Server {
            count: 0,
            listener: listener,
            connections: Slab::new_starting_at(mio::Token(1), 1024),
        }
    }

    fn ready(&mut self,
             poll: &Poll,
             token: Token,
             events: EventSet) {
        debug!("{:?} {:?}", token, events);
        if token == LISTENER {
            while let Ok(Some(socket)) = self.listener.accept() {
                debug!("accepted");
                self.count += 1;
                // socket.0.set_nodelay(true).unwrap();
                let token = self.connections
                    .insert_with(move |_| Connection::new(socket.0))
                    .unwrap();
                self.try_connection(poll, token, EventSet::all());
            }
        } else {
            self.try_connection(poll, token, events);
        }
    }

    fn try_connection(&mut self, poll: &Poll, token: Token, events: EventSet) {
        let res = self.connections[token].ready(events);

        if res.is_err() || self.connections[token].closed {
            if let Err(ref e) = res {
                info!("error: {:?}\non: {:?} {:?}", e,
                       token, self.connections[token].socket);
            }
            debug!("removing");
            self.connections.remove(token);
        } else {
            self.connections[token].reregister(poll, token).unwrap();
        }
    }
}

struct Connection {
    socket: TcpStream,
    input: Vec<u8>,
    output: Output,
    keepalive: bool,
    closed: bool,
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
        }
    }

    fn ready(&mut self, events: EventSet) -> io::Result<()> {
        while events.is_readable() && !self.read_closed {
            let before = self.input.len();
            let eof = try!(read(&mut self.socket, &mut self.input));
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

        if events.is_writable() && self.output.buf.len() > 0 {
            let done = try!(write(&mut self.socket, &mut self.output));
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

    fn reregister(&mut self, poll: &Poll, token: Token) -> io::Result<()> {
        assert!(!self.closed);
        debug!("register");
        let events = if self.output.buf.len() > 0 {
            if self.read_closed {
                EventSet::writable()
            } else {
                EventSet::writable() | EventSet::readable()
            }
        } else {
            assert!(!self.read_closed);
            EventSet::readable()
        };
        let opts = PollOpt::edge() | PollOpt::oneshot();
        if self.first {
            self.first = false;
            poll.register(&self.socket, token, events, opts)
        } else {
            poll.reregister(&self.socket, token, events, opts)
        }
    }
}

fn process(r: &Request) -> Response {
    assert!(r.path() == "/plaintext");
    let mut r = Response::new();
    r.header("Content-Type", "text/plain; charset=UTF-8")
     .body("Hello, World!\r\n");
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
        write!(into, "\
            HTTP/1.1 200 OK\r\n\
            Server: Example\r\n\
            Date: {}\r\n\
            Content-Length: {}\r\n\
        ", time::now().rfc822(), self.response.len()).unwrap();
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

fn read(socket: &mut TcpStream, input: &mut Vec<u8>) -> io::Result<bool> {
    loop {
        match socket.read(unsafe { slice_to_end(input) }) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                let len = input.len();
                unsafe { input.set_len(len + n); }
                return Ok(false)
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                return Ok(false)
            }
            Err(e) => return Err(e),
        }
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

fn write(socket: &mut TcpStream, output: &mut Output) -> io::Result<bool> {
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
                return Ok(false)
            }
            Err(e) => return Err(e),
        }
    }
}

fn main() {
    env_logger::init().unwrap();

    let threads = (0..num_cpus::get()).map(|_| {
        thread::spawn(|| {
            let poll = mio::Poll::new().unwrap();
            let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
            let addr = addr.parse::<SocketAddr>().unwrap();
            let socket = net2::TcpBuilder::new_v4().unwrap();
            socket.reuse_address(true).unwrap();
            socket.reuse_port(true).unwrap();
            socket.bind(&addr).unwrap();
            let listener = socket.listen(2048).unwrap();
            let listener = TcpListener::from_listener(listener, &addr).unwrap();
            poll.register(&listener,
                          LISTENER,
                          EventSet::readable(),
                          PollOpt::level()).unwrap();

            let mut events = Events::new();
            let mut server = Server::new(&listener);
            loop {
                poll.poll(&mut events, None).unwrap();

                for i in 0..events.len() {
                    let event = events.get(i).unwrap();
                    server.ready(&poll, event.token(), event.kind());
                }
            }
        })
    }).collect::<Vec<_>>();

    for thread in threads {
        thread.join().unwrap();
    }
}
