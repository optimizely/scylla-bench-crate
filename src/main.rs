use std::{sync::{atomic::{AtomicI32, Ordering}, Arc}, time::{Instant, Duration}};

use clap::Parser;
use hdrhistogram::Histogram;
use scylla::{SessionBuilder, Session, load_balancing::{TokenAwarePolicy, RoundRobinPolicy}};
use anyhow::Error;
use tokio::{sync::{mpsc}, task::yield_now};

/// open-loop load tester for scylla
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// scylla server to connect to. include the port number.
    #[arg(short, long)]
    server: String,

    /// username if basic password auth should be used
    #[arg(short, long)]
    username: Option<String>,

    /// password if basic password auth should be used
    #[arg(short, long)]
    password: Option<String>,

    /// size of the non-key portion of the row to write
    #[arg(short, long, default_value_t = 1024)]
    blob_size: u32,

    /// constant throughput to write at in requests per second
    #[arg(long)]
    rps: u64,

    /// duration of the test in seconds
    #[arg(long)]
    runtime_s: u64,

    /// upper limit on the concurrency to be used to reach desired rps
    #[arg(long, default_value_t = 4096)]
    max_concurrency: usize,
}

impl Args {
    async fn bench(&self, session: &Session) -> Result<(), Error> {
        let mut blob : Vec<u8> = Vec::new();
        for _i in 0..self.blob_size {
            blob.push(42_u8);
        }

        let concurrency = AtomicI32::new(0);

        // To avoid coordinated omission, we pre-assign a send time to every planned request at test start.
        // This send time is bench_start + delay_between_requests * send_index

        // If the SUT and test are keeping up then this assigned send time will be the real time. If the system
        // isn't keeping up and our max_concurrency is exhausted then send time and real time can arbitrarily diverge.
        // As long as max_concurrency was sufficient to drive the SUT out of its linear scalability region and the
        // test hardware was sufficient to provide max_concurrency, then this divergence is realistic (imagine that the
        // divergence represents requests stuck in the accept queue or waiting in kafka.)
        let bench_start = Instant::now();
        let delay_between_requests = Duration::from_nanos((1e9 / self.rps as f64) as u64);

        let mut spawns = 0i32;
        let (result_sender, mut result_receiver) = mpsc::channel::<(u128, i32)>(self.max_concurrency);
        let prepared = session.prepare(
            "INSERT INTO testrs.foo (id, value) VALUES (?, ?)"
        ).await?;
        let (work_sender, work_receiver) = async_channel::bounded::<i32>(self.max_concurrency);

        tokio_scoped::scope(|s| {
            s.spawn(async {
                let mut hist : Histogram<u64> = hdrhistogram::Histogram::new(2).unwrap();
                eprintln!("runtime_uS,concurrency");
                let mut last_print = Instant::now();
                while let Some((latency, concurrency)) = result_receiver.recv().await {
                    if last_print.elapsed().as_secs() > 1 {
                        eprintln!("{}, {}", latency, concurrency);
                        last_print = Instant::now();
                    }
                    hist.record(latency as u64).unwrap();
                }
                quantiles(&hist, 2, 3).unwrap();
            });
            s.scope(|s| {
                // spawn workers
                for _i in 0..self.max_concurrency {
                    s.spawn(async {
                        while let Result::Ok(idx) = work_receiver.recv().await {
                            let start = bench_start + delay_between_requests * idx as u32;

                            concurrency.fetch_add(1, Ordering::Relaxed);

                            session.execute(&prepared, (idx, &blob)).await.unwrap();
                            
                            let dt = start.elapsed().as_micros();
                            result_sender.send((dt, concurrency.fetch_add(-1, Ordering::Relaxed))).await.unwrap();
                        }
                    });
                }

                // spawn driver
                s.spawn(async {
                    while bench_start.elapsed().as_secs() < self.runtime_s {
                        let target_spawns = ((bench_start.elapsed().as_nanos() * self.rps as u128) as f64 / 1e9).floor() as i32;
                        let next_spawns = target_spawns - spawns;
        
                        for i in 0..next_spawns {
                            work_sender.send(spawns + i).await.unwrap();
                        }
                        spawns += next_spawns;

                        if next_spawns == 0 {
                            yield_now().await;
                        }
                    }
                    work_sender.close();
                });
                
            });
            drop(result_sender);
        });
        
        Result::Ok(())
    }
}


#[tokio::main]
async fn main() -> Result<(), Error> {
    let args : Args = Args::parse();
    let server = &args.server;

    let session = SessionBuilder::new()
        .known_node(server)
        .load_balancing(Arc::new(TokenAwarePolicy::new(Box::new(RoundRobinPolicy::new()))));
    let session = if args.username.is_some() || args.password.is_some() {
        if args.username.is_some() && args.password.is_some() {
            session.user(args.username.as_ref().unwrap(), args.password.as_ref().unwrap())
        } else {
            panic!("if username or password are provided, both must be provided")
        }
    } else {
        session
    };
    let session: Session = session
        .build()
        .await?;

    // delete prior test data
    session.query(
        "DROP KEYSPACE IF EXISTS testrs",
        &[]
    ).await?;

    // set up the benchmark environment
    session.query(
        "CREATE KEYSPACE IF NOT EXISTS testrs WITH REPLICATION = {'class': 'SimpleStrategy', 'replication_factor': 1}",
        &[]
    ).await?;

    session.query(
        "CREATE TABLE testrs.foo (id int primary key, value blob)",
        &[]
    ).await?;

    args.bench(&session).await?;

    // eliminate the test data
    session.query(
        "DROP KEYSPACE IF EXISTS testrs",
        &[]
    ).await?;
    Result::Ok(())
}

fn quantiles(
    hist: &Histogram<u64>,
    quantile_precision: usize,
    ticks_per_half: u32,
) -> Result<(), Error> {
    println!(
        "{:>12} {:>quantile_precision$} {:>quantile_precision$} {:>10} {:>14}",
        "Value",
        "QuantileValue",
        "QuantileIteration",
        "TotalCount",
        "1/(1-Quantile)",
        quantile_precision = quantile_precision + 2 // + 2 from leading "0." for numbers
    );
    let mut sum = 0;
    for v in hist.iter_quantiles(ticks_per_half) {
        sum += v.count_since_last_iteration();
        if v.quantile_iterated_to() < 1.0 {
            println!(
                "{:12} {:1.*} {:1.*} {:10} {:14.2}",
                v.value_iterated_to(),
                quantile_precision,
                v.quantile(),
                quantile_precision,
                v.quantile_iterated_to(),
                sum,
                1_f64 / (1_f64 - v.quantile_iterated_to())
            );
        } else {
            println!(
                "{:12} {:1.*} {:1.*} {:10} {:>14}",
                v.value_iterated_to(),
                quantile_precision,
                v.quantile(),
                quantile_precision,
                v.quantile_iterated_to(),
                sum,
                "∞"
            );
        }
    }
        
    Result::Ok(())
}