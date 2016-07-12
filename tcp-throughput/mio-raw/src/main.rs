extern crate mio;

use std::net::SocketAddr;
use std::sync::atomic::*;
use std::thread;
use std::time::Duration;

use mio::{Poll, EventSet, Token, PollOpt, TryRead};
use mio::tcp::TcpStream;

static AMT: AtomicUsize = ATOMIC_USIZE_INIT;

fn main() {
    thread::spawn(|| {
        loop {
            thread::sleep(Duration::new(1, 0));
            println!("{}", AMT.swap(0, Ordering::SeqCst));
        }
    });

    let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    let mut conn = TcpStream::connect(&addr).unwrap();

    let mut poll = Poll::new().unwrap();

    poll.register(&conn,
                  Token(0),
                  EventSet::readable(),
                  PollOpt::edge()).unwrap();

    let mut buf = [0; 64 * 1024];
    loop {
        poll.poll(None).unwrap();

        while let Some(n) = conn.try_read(&mut buf).unwrap() {
            AMT.fetch_add(n, Ordering::SeqCst);
        }
    }
}
