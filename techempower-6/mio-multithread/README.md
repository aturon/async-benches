# mio-multithread

This is a cross-platform implementation of using mio in multiple threads at
once. This is not the same as the mio-multithread-unix benchmark which leverages
`SO_REUSEPORT`, instead this benchmark has multiple threads all listening on the
same event loop for events. Events are then dispatched to threads at the OS's
discretion.

There's a bit of "atomic trickery" here so... it's not the greatest example
code. Feel free to reach out to @alexcrichton though if you'd like to know more!

Currently gets the same perf as `SO_REUSEPORT` on Unix and gets better perf on
Windows than mio-singlethread, but too many threads on Windows will completely
tank performance to worse than mio-singlethread. Not much investigation has been
done as to why, but the mio implementation on Windows uses a lot of `Mutex`
still which is perhaps the problem.
