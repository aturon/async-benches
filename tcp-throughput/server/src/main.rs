use std::env;
use std::io::*;
use std::net::*;
use std::sync::atomic::*;
use std::thread;
use std::time::Duration;

static AMT: AtomicUsize = ATOMIC_USIZE_INIT;

fn main() {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:12345".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();
    let l = TcpListener::bind(&addr).unwrap();
    println!("listening...");
    let b = [0; 64 * 1024];
    thread::spawn(|| {
        loop {
            thread::sleep(Duration::new(1, 0));
            println!("{}", AMT.swap(0, Ordering::SeqCst));
        }
    });
    loop {
        let mut c = l.accept().unwrap().0;
        println!("got a connection, writing some data to it");
        while let Ok(n) = c.write(&b) {
            AMT.fetch_add(n, Ordering::SeqCst);
        }
        println!("connection is gone, listening for another");
    }
}
