# scylla-bench: constant cate (CRrate)

This simulates a constant rate load and delivers it to scylla with variable concurrency. The name is also a bad Rust pun.

This is an open-loop scylla load testing tool developed in the creation of [this talk](https://docs.google.com/presentation/d/1FPBTbZJEx9xKARambDyNLLzGN2AXiukXx-vsSBZdk_I/edit?usp=sharing). It is intended to complement the scylla provided closed-loop testing tool, [scylla-bench](https://github.com/scylladb/scylla-bench).

In closed-loop load testing (not this), we choose concurrency (perhaps called "users" in some implementations) and discover the throughput that this concurrency implied. In open-loop load testing (what this does), we choose throughput and let the tool find the dynamic concurrency that yields that throughput.

# Usage 

The required arguments are `--server`, `--rps`, and `--runtime-s`. We recommend choosing a `--runtime-s` that is sufficiently long to observe scylla's major performance states (the various intensities of compaction). At reasonable throughput and cluster sizes this is typically about an hour. A test that is too short may lead to the false conclusion that the requested throughput can be achieved with stability.

As the program runs it will print the elapsed time (in $\mu S$) and the current concurrency to stderr. At the conclusion of the test an hdrhistogram of request latency will be printed to stdout. If the reported concurrency reaches the limit specified by `max-concurrency` (4096 by default) then either Scylla or this tester are saturated and the request latency values are not reliable. If Scylla is saturated and you're capacity planning, [in Gil's words, you've crashed your little red car and it's time to slow down and find the safe speed.](https://www.youtube.com/watch?v=lJ8ydIuPFeU) The accuracy with which we measure the destruction at saturation isn't important.

You can determine that Scylla is saturated via your Scylla observability. For example, if Scylla's SSDs or CPUs are saturated then Scylla is saturated. If you suspect that this tester itself is saturated then reasonable interventions (in order of recommendation) are:
* Run the test on a more privileged network. Closer to Scylla
* Run the test from multiple nodes, splitting your target throughput between the nodes
* Increase max-concurrency

```
$ ./scylla-bench-crate --help

open-loop load tester for scylla

Usage: scylla-bench-crate [OPTIONS] --server <SERVER> --rps <RPS> --runtime-s <RUNTIME_S>

Options:
  -s, --server <SERVER>
          scylla server to connect to. include the port number
  -u, --username <USERNAME>
          username if basic password auth should be used
  -p, --password <PASSWORD>
          password if basic password auth should be used
  -b, --blob-size <BLOB_SIZE>
          size of the non-key portion of the row to write [default: 1024]
      --rps <RPS>
          constant throughput to write at in requests per second
      --runtime-s <RUNTIME_S>
          duration of the test in seconds
      --max-concurrency <MAX_CONCURRENCY>
          upper limit on the concurrency to be used to reach desired rps [default: 4096]
  -h, --help
          Print help
  -V, --version
          Print version
```

# Installation

TL;DR: Do the normal rust stuff.

Long version
1. Install rust with rustup if you haven't.
2. In the project root run `cargo build --release`
3. The thing you want is now in `./target/release/scylla-bench-crate`
