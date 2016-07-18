extern crate time;
extern crate mio;
extern crate httparse;

use std::env;
use std::fmt::Write;
use std::net::SocketAddr;

use mio::{Handler, EventLoop, Token, EventSet, PollOpt, TryRead, TryWrite};
use mio::tcp::*;
use mio::util::Slab;

const LISTENER: mio::Token = mio::Token(0);

struct Server {
    count: u64,
    listener: TcpListener,
    connections: Slab<Connection>,
}

impl Server {
    fn new(listener: TcpListener) -> Server {
        Server {
            count: 0,
            listener: listener,
            connections: Slab::new_starting_at(mio::Token(1), 1024),
        }
    }
}

impl Handler for Server {
    type Timeout = ();
    type Message = ();

    fn ready(&mut self, event_loop: &mut EventLoop<Server>, token: Token, events: EventSet) {
        println!("token {:?}", token);

        if token == LISTENER {
            if let Ok(Some(socket)) = self.listener.accept() {
                self.count += 1;
                println!("...connection {}", self.count);

                let token = self.connections
                    .insert_with(move |token| Connection::new(token, socket.0))
                    .unwrap();

                event_loop.register(
                    &self.connections[token].socket,
                    token,
                    EventSet::readable(),
                    PollOpt::edge() | PollOpt::oneshot()).unwrap();
            }
        } else {
            self.connections[token].ready(event_loop, token, events);

            if self.connections[token].is_closed() {
                println!("...closing");
                self.connections.remove(token);
            }
        }
    }
}

struct Connection {
    socket: TcpStream,
    token: Token,
    state: State,
}

type Input = Vec<u8>;

struct Output {
    buf: Vec<u8>,
    pos: usize,
}

enum State {
    Request(Input),
    Response(Output),
    Closed,
}

impl Connection {
    fn new(token: Token, socket: TcpStream) -> Connection {
        Connection {
            token: token,
            socket: socket,
            state: State::Request(Input::new()),
        }
    }

    fn ready(&mut self, event_loop: &mut EventLoop<Server>, token: Token, events: EventSet) {
        let move_forward = match self.state {
            State::Request(ref mut input) => read(&mut self.socket, input, event_loop),
            State::Response(ref mut output) => write(&mut self.socket, output, event_loop),
            State::Closed => unreachable!(),
        };
        if move_forward {
            self.move_forward();
        }
        self.reregister(event_loop, token);
    }

    fn move_forward(&mut self) {
        println!("state change");

        self.state = match self.state {
            State::Request(_) => {
                let mut r = Response::new();
                r.header("Content-Type", "text/plain")
                    .header("Content-Lenth", "15")
                    .header("Server", "wut")
                    .header("Date", &time::now().rfc822().to_string())
                    .body("Hello, World!");
                State::Response(Output { buf: r.to_bytes(), pos: 0 })
            }
            State::Response(_) => State::Closed,
            State::Closed => unreachable!(),
        };
    }

    fn is_closed(&self) -> bool {
        match self.state {
            State::Closed => true,
            _ => false,
        }
    }

    fn reregister(&mut self, event_loop: &mut EventLoop<Server>, token: Token) {
        match self.state {
            State::Request(_) => {
                event_loop.reregister(
                    &self.socket,
                    token,
                    EventSet::readable(),
                    PollOpt::edge() | PollOpt::oneshot()).unwrap()
            }
            State::Response(_) => {
                event_loop.reregister(
                    &self.socket,
                    token,
                    EventSet::writable(),
                    PollOpt::edge() | PollOpt::oneshot()).unwrap()
            }
            State::Closed => {}
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        println!("...dropping connection");
    }
}

struct Request {
    method: String,
    path: String,
    version: u8,
    headers: Vec<(String, Vec<u8>)>,
}

fn parse(buf: &[u8]) -> Option<Request> {
    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut r = httparse::Request::new(&mut headers);
    let status = match r.parse(&buf) {
        Ok(status) => status,
        Err(e) => {
            return None
        }
    };
    match status {
        httparse::Status::Complete(amt) => {
            assert_eq!(amt, buf.len());
            Some(Request {
                method: r.method.unwrap().to_string(),
                path: r.path.unwrap().to_string(),
                version: r.version.unwrap(),
                headers: r.headers.iter().map(|h| {
                    (h.name.to_string(), h.value.to_owned())
                }).collect(),
            })
        }
        httparse::Status::Partial => None
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

    // first, inefficient stab: just build a single buffer big enough to hold entire response
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = String::new();
        write!(bytes, "HTTP/1.1 200 OK\r\n");
        for &(ref k, ref v) in &self.headers {
            write!(bytes, "{}: {}\r\n", k, v).unwrap();
        }
        write!(bytes, "\r\n").unwrap();
        write!(bytes, "{}", self.response).unwrap();
        bytes.into_bytes()
    }
}

fn read(socket: &mut TcpStream, input: &mut Input, event_loop: &mut EventLoop<Server>) -> bool {
    match socket.try_read_buf(input).unwrap() {
        // spurious notification
        None => {
            false
        }

        // move forward in case of unexpected eof
        Some(0) => {
            true
        }

        // read `n`, attempt parse
        Some(n) => {
            parse(input).is_some()
        }
    }
}

fn write(socket: &mut TcpStream, output: &mut Output, event_loop: &mut EventLoop<Server>) -> bool {
    match socket.try_write(&output.buf[output.pos..]).unwrap() {
        // spurious
        None => {
            false
        }

        Some(n) => {
            output.pos += n;
            output.pos == output.buf.len()
        }
    }
}

// ab -c 8 -n 10000 localhost:8080/plaintext
fn main() {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();
    let listener = TcpListener::bind(&addr).unwrap();
    let mut event_loop = mio::EventLoop::new().unwrap();
    event_loop.register(&listener,
                        LISTENER,
                        EventSet::readable(),
                        PollOpt::level())
        .unwrap();
    let mut server = Server::new(listener);
    event_loop.run(&mut server).unwrap();
}
