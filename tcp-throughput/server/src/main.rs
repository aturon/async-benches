use std::net::*;
use std::io::*;

fn main() {
    let l = TcpListener::bind("127.0.0.1:12345").unwrap();
    println!("listening...");
    let b = [0; 64 * 1024];
    loop {
        let mut c = l.accept().unwrap().0;
        println!("got a connection, writing some data to it");
        while c.write(&b).is_ok() {}
        println!("connection is gone, listening for another");
    }
}
