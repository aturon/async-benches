# mio-singlethread

Single-threaded performance of the `/plaintext` benchmark from techempower which
is just written using raw mio. Attempts to be as fast as possible while only
using mio and not doing *too* many contortions.

```
cargo run --release
curl http://127.0.0.1:8080/plaintext
wrk --script ./pipelined_get.lua --latency -d 30s -t 40 -c 760 \
  http://127.0.0.1:8080/plaintext -- 32
```

Currently gets faster by:

* Implementing keepalive
* Implementing HTTP pipelining
* Buffering both requests and responses
* Not allocating well-known headers for responses
