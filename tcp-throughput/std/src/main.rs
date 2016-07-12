use std::io::Read;
use std::net::*;
use std::sync::atomic::*;
use std::thread;
use std::time::Duration;

static AMT: AtomicUsize = ATOMIC_USIZE_INIT;

fn main() {
    thread::spawn(|| {
        loop {
            thread::sleep(Duration::new(1, 0));
            println!("{}", AMT.swap(0, Ordering::SeqCst));
        }
    });

    let mut c = TcpStream::connect("127.0.0.1:12345").unwrap();
    let mut b = [0; 64 * 1024];
    loop {
        match c.read(&mut b).unwrap() {
            0 => break,
            n => { AMT.fetch_add(n, Ordering::SeqCst); }
        }
    }
}