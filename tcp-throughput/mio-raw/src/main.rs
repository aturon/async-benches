extern crate mio;

use std::env;
use std::net::SocketAddr;
use std::sync::atomic::*;
use std::thread;
use std::time::Duration;

use mio::{Poll, Events, EventSet, Token, PollOpt, TryRead};
use mio::tcp::TcpStream;

static AMT: AtomicUsize = ATOMIC_USIZE_INIT;

fn main() {
    thread::spawn(|| {
        loop {
            thread::sleep(Duration::new(1, 0));
            println!("{}", AMT.swap(0, Ordering::SeqCst));
        }
    });

    let addr = env::args().nth(1).unwrap_or("127.0.0.1:12345".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();
    let mut conn = TcpStream::connect(&addr).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::new();

    poll.register(&conn,
                  Token(0),
                  EventSet::readable(),
                  PollOpt::edge()).unwrap();

    let mut buf = [0; 64 * 1024];
    loop {
        poll.poll(&mut events, None).unwrap();

        while let Some(n) = conn.try_read(&mut buf).unwrap() {
            AMT.fetch_add(n, Ordering::SeqCst);
        }
    }
}
