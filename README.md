# scylla-bench Constant Rate (CRrate)

(it seemed a shame to not make a rust pun)

This is a very simple version of scylla bench designed to benchmark under constant throughput conditions which better simulate an internet workload than a static concurrency configuration.

Classic load testing involves choosing a number of users (--concurrency in scylla bench). These users each make a request, wait patiently for it to be resolved, and then make their next request. This has a lovely self-stabilizing property in that if the SUT slows down so do the users. Unfortunately, this is also nothing like the real world. In the real world, there are unlimited users and they don't slow down just because your system slowed down. More often than not they speed up because they start impatiently slamming the refresh button.

This simulates a constant rate load and delivers it to scylla with unbounded concurrency.