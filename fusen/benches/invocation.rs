//! Direct single-attempt socket matrix used by the 0.9 release gate.

use fusen_rs::{
    ClientConfig, ClientRuntime, Error, Response, RetryConfig, Server, ServerConfig,
    contract::{EndpointCapabilities, HttpBindingId, HttpVersionPolicy, HttpVersionSet},
    interface,
};
use std::{
    env,
    error::Error as StdError,
    hint::black_box,
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::task::JoinSet;

const DEFAULT_WARMUP_ITERATIONS: usize = 500;
const DEFAULT_SMALL_ITERATIONS: usize = 10_000;
const DEFAULT_LARGE_ITERATIONS: usize = 1_000;
const LARGE_PAYLOAD_BYTES: usize = 64 * 1024;
const CONCURRENCIES: [usize; 2] = [1, 100];

#[interface(name = "benchmark")]
trait BenchmarkService {
    #[method(method = "POST", path = "/benchmark/echo")]
    async fn echo(&self, #[param(body)] value: String) -> Result<Response<String>, Error>;
}

struct BenchmarkServiceImpl;

impl BenchmarkService for BenchmarkServiceImpl {
    async fn echo(&self, value: String) -> Result<Response<String>, Error> {
        Ok(Response::new(value))
    }
}

#[derive(Clone, Copy)]
enum BenchmarkTransport {
    Http1,
    H2c,
}

impl BenchmarkTransport {
    const ALL: [Self; 2] = [Self::Http1, Self::H2c];

    const fn id(self) -> &'static str {
        match self {
            Self::Http1 => "http1",
            Self::H2c => "h2c",
        }
    }

    const fn policy(self) -> HttpVersionPolicy {
        match self {
            Self::Http1 => HttpVersionPolicy::Http1,
            Self::H2c => HttpVersionPolicy::H2c,
        }
    }
}

struct BenchmarkParameters {
    warmup_iterations: usize,
    small_iterations: usize,
    large_iterations: usize,
}

impl BenchmarkParameters {
    fn from_env() -> Result<Self, io::Error> {
        let parameters = Self {
            warmup_iterations: positive_env(
                "FUSEN_BENCH_WARMUP_ITERATIONS",
                DEFAULT_WARMUP_ITERATIONS,
            )?,
            small_iterations: positive_env(
                "FUSEN_BENCH_SMALL_ITERATIONS",
                DEFAULT_SMALL_ITERATIONS,
            )?,
            large_iterations: positive_env(
                "FUSEN_BENCH_LARGE_ITERATIONS",
                DEFAULT_LARGE_ITERATIONS,
            )?,
        };
        if parameters.small_iterations < 100 || parameters.large_iterations < 100 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "measured benchmark cases require at least 100 iterations for p99",
            ));
        }
        Ok(parameters)
    }
}

struct Payload {
    label: &'static str,
    value: Arc<String>,
    iterations: usize,
}

#[derive(Default)]
struct TaskResult {
    latency_ns: Vec<u64>,
    bytes: u64,
    errors: u64,
    first_error: Option<String>,
}

struct CaseResult {
    iterations: usize,
    bytes: u64,
    errors: u64,
    duration: Duration,
    p50_ns: u64,
    p99_ns: u64,
    first_error: Option<String>,
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("benchmark runtime must build");
    runtime
        .block_on(run())
        .expect("direct invocation benchmark matrix must complete");
}

async fn run() -> Result<(), Box<dyn StdError>> {
    let parameters = BenchmarkParameters::from_env()?;
    println!(
        "benchmark-parameters warmup_iterations={} small_iterations={} large_iterations={}",
        parameters.warmup_iterations, parameters.small_iterations, parameters.large_iterations,
    );

    let binding = HttpBindingId::default();
    let capabilities = EndpointCapabilities::new(HttpVersionSet::ALL, [binding.clone()], true)?;
    let server_config = ServerConfig::builder()
        .capabilities(capabilities.clone())
        .build()?;
    let server = Server::builder("127.0.0.1:0")
        .config(server_config)
        .interface(BenchmarkServiceServer::new(BenchmarkServiceImpl))
        .build()?
        .start()
        .await?;
    let server_url = format!("http://{}", server.local_addr());

    let payloads = [
        Payload {
            label: "small",
            value: Arc::new("payload".to_owned()),
            iterations: parameters.small_iterations,
        },
        Payload {
            label: "64k",
            value: Arc::new("x".repeat(LARGE_PAYLOAD_BYTES)),
            iterations: parameters.large_iterations,
        },
    ];
    let mut benchmark_errors = 0_u64;

    for transport in BenchmarkTransport::ALL {
        let client_config = ClientConfig::builder()
            .retry(RetryConfig::builder().max_attempts(1).build()?)
            .build()?;
        let client_runtime = ClientRuntime::builder().config(client_config).build()?;
        let client = BenchmarkServiceClient::builder(&client_runtime)
            .direct(&server_url)
            .binding(binding.clone())
            .http_version_policy(transport.policy())
            .direct_capabilities(capabilities.clone())
            .connect()
            .await?;

        for concurrency in CONCURRENCIES {
            for payload in &payloads {
                warm_up(
                    &client,
                    Arc::clone(&payload.value),
                    parameters.warmup_iterations,
                    concurrency,
                )
                .await?;
                let result = run_case(
                    &client,
                    Arc::clone(&payload.value),
                    payload.iterations,
                    concurrency,
                )
                .await?;
                benchmark_errors = benchmark_errors.saturating_add(result.errors);
                print_result(&binding, transport, concurrency, payload, &result);
            }
        }

        drop(client);
        client_runtime.shutdown().await?;
    }

    server.shutdown().await?;
    if benchmark_errors != 0 {
        return Err(io::Error::other(format!(
            "benchmark matrix observed {benchmark_errors} failed request(s)"
        ))
        .into());
    }
    Ok(())
}

async fn warm_up(
    client: &BenchmarkServiceClient,
    payload: Arc<String>,
    iterations: usize,
    concurrency: usize,
) -> Result<(), Box<dyn StdError>> {
    let result = execute_requests(client, payload, iterations, concurrency).await?;
    if result.errors != 0 {
        return Err(io::Error::other(format!(
            "benchmark warmup observed {} failed request(s): {}",
            result.errors,
            result.first_error.as_deref().unwrap_or("unknown error")
        ))
        .into());
    }
    Ok(())
}

async fn run_case(
    client: &BenchmarkServiceClient,
    payload: Arc<String>,
    iterations: usize,
    concurrency: usize,
) -> Result<CaseResult, tokio::task::JoinError> {
    let started = Instant::now();
    let mut result = execute_requests(client, payload, iterations, concurrency).await?;
    let duration = started.elapsed();
    result.latency_ns.sort_unstable();
    let p50_ns = percentile(&result.latency_ns, 50);
    let p99_ns = percentile(&result.latency_ns, 99);
    Ok(CaseResult {
        iterations,
        bytes: result.bytes,
        errors: result.errors,
        duration,
        p50_ns,
        p99_ns,
        first_error: result.first_error,
    })
}

async fn execute_requests(
    client: &BenchmarkServiceClient,
    payload: Arc<String>,
    iterations: usize,
    concurrency: usize,
) -> Result<TaskResult, tokio::task::JoinError> {
    let mut tasks = JoinSet::new();
    for worker in 0..concurrency {
        let worker_iterations =
            iterations / concurrency + usize::from(worker < iterations % concurrency);
        if worker_iterations == 0 {
            continue;
        }
        let client = client.clone();
        let payload = Arc::clone(&payload);
        tasks.spawn(async move {
            let mut result = TaskResult {
                latency_ns: Vec::with_capacity(worker_iterations),
                ..TaskResult::default()
            };
            for _ in 0..worker_iterations {
                let request = payload.as_ref().clone();
                let started = Instant::now();
                match client.echo(request).await {
                    Ok(response) if response.body() == payload.as_ref() => {
                        black_box(response.body());
                        result
                            .latency_ns
                            .push(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
                        result.bytes = result
                            .bytes
                            .saturating_add((payload.len() as u64).saturating_mul(2));
                    }
                    Ok(_) => {
                        result.errors = result.errors.saturating_add(1);
                        result.first_error.get_or_insert_with(|| {
                            "echo response did not match request".to_owned()
                        });
                    }
                    Err(error) => {
                        result.errors = result.errors.saturating_add(1);
                        result.first_error.get_or_insert_with(|| error.to_string());
                    }
                }
            }
            result
        });
    }

    let mut combined = TaskResult::default();
    while let Some(result) = tasks.join_next().await {
        let mut task = result?;
        combined.latency_ns.append(&mut task.latency_ns);
        combined.bytes = combined.bytes.saturating_add(task.bytes);
        combined.errors = combined.errors.saturating_add(task.errors);
        if combined.first_error.is_none() {
            combined.first_error = task.first_error;
        }
    }
    Ok(combined)
}

fn percentile(samples: &[u64], percentile: usize) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let rank = samples
        .len()
        .saturating_mul(percentile)
        .div_ceil(100)
        .saturating_sub(1);
    samples[rank.min(samples.len() - 1)]
}

fn print_result(
    binding: &HttpBindingId,
    transport: BenchmarkTransport,
    concurrency: usize,
    payload: &Payload,
    result: &CaseResult,
) {
    let case = format!("{}-c{concurrency}-{}", transport.id(), payload.label);
    let successful = result.iterations as u64 - result.errors.min(result.iterations as u64);
    let qps = successful as f64 / result.duration.as_secs_f64();
    println!(
        "benchmark-result case={case} binding={} transport={} concurrency={concurrency} \
         payload={} payload_bytes={} iterations={} bytes={} errors={} duration_ns={} \
         qps={qps:.3} p50_ns={} p99_ns={}",
        binding.as_str(),
        transport.id(),
        payload.label,
        payload.value.len(),
        result.iterations,
        result.bytes,
        result.errors,
        result.duration.as_nanos(),
        result.p50_ns,
        result.p99_ns,
    );
    if let Some(error) = result.first_error.as_deref() {
        eprintln!("benchmark case {case} first error: {error}");
    }
}

fn positive_env(name: &str, default: usize) -> Result<usize, io::Error> {
    let value = match env::var(name) {
        Ok(value) => value.parse::<usize>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} must be a positive integer: {error}"),
            )
        })?,
        Err(env::VarError::NotPresent) => default,
        Err(error) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cannot read {name}: {error}"),
            ));
        }
    };
    if value == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be greater than zero"),
        ));
    }
    Ok(value)
}
