extern crate iron;

use std::net::SocketAddr;
use std::env;

use iron::prelude::*;
use iron::Protocol;
use iron::mime::{self, Mime};
use iron::mime::TopLevel::Text;
use iron::mime::SubLevel::Plain;
use iron::status;
use iron::modifiers::Header;
use iron::headers::{ContentType, Server};

fn main() {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();
    Iron::new(|_: &mut Request| {
        let data = (mime::Attr::Charset, mime::Value::Utf8);
        let content_type = ContentType(Mime(Text, Plain, vec![data]));
        Ok(Response::with((Header(content_type),
                           Header(Server("Example".to_string())),
                           status::Ok,
                           "Hello, World!")))
    }).listen_with(&addr, 1, Protocol::Http, None).unwrap();
}
