# mio-multithread-unix

This is the same as the mio-singlethread benchmark, except it leverages
`SO_REUSEPORT` and `SO_REUSEADDR` to have multiple event loops all listening on
the same socket. This is currently a Unix-specific option and isn't available on
Windows.

The performance should be greater than mio-singlethread on multicore machines,
and otherwise the code is intended to be the same.

See the mio-singlethread README for how to run
