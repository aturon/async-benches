use std::env;
use std::io::*;
use std::net::*;

fn main() {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:12345".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();
    let l = TcpListener::bind(&addr).unwrap();
    println!("listening...");
    let b = [0; 64 * 1024];
    loop {
        let mut c = l.accept().unwrap().0;
        println!("got a connection, writing some data to it");
        while c.write(&b).is_ok() {}
        println!("connection is gone, listening for another");
    }
}
