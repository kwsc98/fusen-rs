//! Direct single-attempt h2c latency baseline used by the 0.9 release gate.

use fusen_rs::{ClientRuntime, RpcError, Server, WireProtocol, service};
use std::{error::Error, hint::black_box, time::Instant};

const WARMUP_ITERATIONS: usize = 500;
const MEASURED_ITERATIONS: usize = 10_000;

#[service(name = "benchmark")]
trait BenchmarkService {
    #[method(idempotency = "safe")]
    async fn echo(&self, value: String) -> Result<String, RpcError>;
}

struct BenchmarkServiceImpl;

impl BenchmarkService for BenchmarkServiceImpl {
    async fn echo(&self, value: String) -> Result<String, RpcError> {
        Ok(value)
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("benchmark runtime must build");
    runtime
        .block_on(run())
        .expect("direct single-attempt benchmark must complete");
}

async fn run() -> Result<(), Box<dyn Error>> {
    let server = Server::builder("127.0.0.1:0")
        .service(BenchmarkServiceServer::new(BenchmarkServiceImpl))
        .build()?
        .start()
        .await?;
    let runtime = ClientRuntime::builder().build()?;
    let client = BenchmarkServiceClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .protocol(WireProtocol::FusenV1)
        .connect()
        .await?;

    for _ in 0..WARMUP_ITERATIONS {
        black_box(client.echo("warmup".to_owned()).await?);
    }

    let mut samples = Vec::with_capacity(MEASURED_ITERATIONS);
    let measured = Instant::now();
    for _ in 0..MEASURED_ITERATIONS {
        let started = Instant::now();
        black_box(client.echo("payload".to_owned()).await?);
        samples.push(started.elapsed());
    }
    let total = measured.elapsed();
    samples.sort_unstable();
    let p50 = samples[MEASURED_ITERATIONS / 2];
    let p99 = samples[MEASURED_ITERATIONS * 99 / 100];
    println!(
        "direct/fusen-v1 iterations={MEASURED_ITERATIONS} mean_ns={} p50_ns={} p99_ns={}",
        total.as_nanos() / MEASURED_ITERATIONS as u128,
        p50.as_nanos(),
        p99.as_nanos(),
    );

    drop(client);
    runtime.shutdown().await?;
    server.shutdown().await?;
    Ok(())
}
