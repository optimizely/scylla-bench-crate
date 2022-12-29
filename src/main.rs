use std::{sync::{atomic::{AtomicI32, Ordering}, Arc}, time::{Instant, Duration}};

use clap::Parser;
use hdrhistogram::Histogram;
use scylla::{SessionBuilder, Session, load_balancing::{TokenAwarePolicy, RoundRobinPolicy}};
use anyhow::Error;
use tokio::sync::mpsc;


#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    server: String,

    #[arg(short, long)]
    username: String,

    #[arg(short, long)]
    password: String,

    #[arg(short, long)]
    blob_size: u32,

    #[arg(long)]
    rps: u64,

    #[arg(long)]
    runtime_s: u64,

    #[arg(long)]
    no_prepared: bool,
}

impl Args {
    async fn bench(&self, session: &Session) -> Result<(), Error> {
        let mut blob : Vec<u8> = Vec::new();
        for _i in 0..self.blob_size {
            blob.push(42_u8);
        }

        let row_id = AtomicI32::new(0);

        let bench_start = Instant::now();
        let delay_between_requests = Duration::from_nanos((1e9 / self.rps as f64) as u64);

        let mut spawns = 0u64;
        let (sender, mut receiver) = mpsc::channel::<(u128, u64)>(128);
        let prepared = session.prepare(
            "INSERT INTO testrs.foo (id, value) VALUES (?, ?)"
        ).await?;

        tokio_scoped::scope(|s| {
            s.spawn(async {
                let mut hist : Histogram<u64> = hdrhistogram::Histogram::new(2).unwrap();
                println!("runtime_uS,concurrency");
                let mut last_print = Instant::now();
                while let Some((latency, concurrency)) = receiver.recv().await {
                    if last_print.elapsed().as_secs() > 1 {
                        eprintln!("{}, {}", latency, concurrency);
                        last_print = Instant::now();
                    }
                    hist.record(latency as u64).unwrap();
                }
                quantiles(&hist, 2, 3).unwrap();
            });
            while bench_start.elapsed().as_secs() < self.runtime_s {
                let target_spawns = ((bench_start.elapsed().as_nanos() * self.rps as u128) as f64 / 1e9).floor() as u64;
                let next_spawns = target_spawns - spawns;
                spawns += next_spawns;

                s.scope(|s| {
                    for _i in 0..next_spawns {
                        s.spawn(async {
                            //let blob : &[u8] = &blob;
                            let idx = row_id.fetch_add(1, Ordering::Relaxed);
                            let start = bench_start + delay_between_requests * idx as u32;
                            if self.no_prepared {
                                session.query(
                                    "INSERT INTO testrs.foo (id, value) VALUES (?, ?)",
                                    (idx, &blob)
                                ).await.unwrap();
                            } else {
                                session.execute(&prepared, (idx, &blob)).await.unwrap();
                            }
                            
                            let dt = start.elapsed().as_micros();
                            sender.send((dt, next_spawns)).await.unwrap();
                        });
                    }
                });
            }
            drop(sender);
        });
        
        Result::Ok(())
    }
}


#[tokio::main]
async fn main() -> Result<(), Error> {
    let args : Args = Args::parse();
    let server = &args.server;
    let server = format!("{server}:9042");

    let session: Session = SessionBuilder::new()
        .known_node(server)
        .load_balancing(Arc::new(TokenAwarePolicy::new(Box::new(RoundRobinPolicy::new()))))
        .user(&args.username, &args.password)
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