# Raw TCP throughput performance

Collection of programs which just connect to a server and attempt to read as
much data off it as possible.

First, run the server

```
cargo run --release --manifest-path server/Cargo.toml
```

Then, run a client

```
cargo run --release --manifest-path mio-raw/Cargo.toml
```
