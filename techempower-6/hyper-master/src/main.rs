#[macro_use]
extern crate log;
extern crate hyper;
extern crate env_logger;
extern crate mime;

use std::env;
use std::net::SocketAddr;

use mime::Mime;
use mime::TopLevel::Text;
use mime::SubLevel::Plain;

use hyper::{RequestUri, Decoder, Encoder, Next};
use hyper::header::{ContentLength, ContentType, Server as ServerHeader};
use hyper::net::HttpStream;
use hyper::server::{Server, Handler, Request, Response, HttpListener};

struct Plaintext;

const INDEX: &'static [u8] = b"Hello, World!";

impl Handler<HttpStream> for Plaintext {
    fn on_request(&mut self, req: Request<HttpStream>) -> Next {
        match *req.uri() {
            RequestUri::AbsolutePath(ref path) => {
                assert_eq!(&path[..], "/plaintext");
                Next::write()
            }
            _ => panic!("wrong uri?"),
        }
    }

    fn on_request_readable(&mut self,
                           _transport: &mut Decoder<HttpStream>) -> Next {
        panic!()
    }

    fn on_response(&mut self, res: &mut Response) -> Next {
        let data = (mime::Attr::Charset, mime::Value::Utf8);
        res.headers_mut().set(ContentType(Mime(Text, Plain, vec![data])));
        res.headers_mut().set(ContentLength(INDEX.len() as u64));
        res.headers_mut().set(ServerHeader("Example".to_string()));
        Next::write()
    }

    fn on_response_writable(&mut self, transport: &mut Encoder<HttpStream>) -> Next {
        transport.write(INDEX).unwrap();
        Next::end()
    }
}

// TODO: this program appears to not work with pipelined wrk requests?

fn main() {
    env_logger::init().unwrap();

    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();

    let listener = HttpListener::bind(&addr).unwrap();
    let mut handles = Vec::new();

    for _ in 0..8 {
        let listener = listener.try_clone().unwrap();
        handles.push(::std::thread::spawn(move || {
            Server::new(listener)
                .handle(|_| Plaintext).unwrap();
        }));
    }
    println!("Listening on http://127.0.0.1:3000");

    for handle in handles {
        handle.join().unwrap();
    }
}
