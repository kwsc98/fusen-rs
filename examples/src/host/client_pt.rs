//! Direct HTTP/1.1 and h2c benchmark client.

use std::{env, io, time::Instant};

use examples::{DemoServiceClient, RequestDto};
use fusen_rs::{ClientRuntime, contract::WireProtocol};
use tokio::task::JoinSet;

const DEFAULT_CONCURRENCIES: &[usize] = &[1, 100];
const DEFAULT_REQUESTS_PER_TASK: usize = 10_000;
const DEFAULT_ROUNDS: usize = 5;
const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8081";
const REQUEST_VALUE: &str = "benchmark";

#[derive(Clone, Copy, PartialEq, Eq)]
enum BenchmarkProtocol {
    Http1,
    Http2,
}

impl BenchmarkProtocol {
    fn label(self) -> &'static str {
        match self {
            Self::Http1 => "HTTP/1.1",
            Self::Http2 => "HTTP/2",
        }
    }

    fn wire_protocol(self) -> WireProtocol {
        match self {
            Self::Http1 => WireProtocol::SpringCloudV1,
            Self::Http2 => WireProtocol::FusenV1,
        }
    }
}

struct BenchmarkConfig {
    concurrency: usize,
    requests_per_task: usize,
    planned_requests: usize,
    server_url: String,
    request_body_bytes: u64,
}

#[derive(Default)]
struct TaskStats {
    succeeded: u64,
    failed: u64,
    response_body_bytes: u64,
    first_error: Option<String>,
}

struct BenchmarkResult {
    protocol: BenchmarkProtocol,
    stats: TaskStats,
    elapsed_seconds: f64,
    request_body_bytes: u64,
}

struct BenchmarkSummary {
    protocol: BenchmarkProtocol,
    concurrency: usize,
    median_qps: f64,
    median_successful_qps: f64,
    median_throughput_mib: f64,
}

impl BenchmarkResult {
    fn completed(&self) -> u64 {
        self.stats.succeeded + self.stats.failed
    }

    fn request_bytes(&self) -> u64 {
        self.completed().saturating_mul(self.request_body_bytes)
    }

    fn total_bytes(&self) -> u64 {
        self.request_bytes()
            .saturating_add(self.stats.response_body_bytes)
    }

    fn qps(&self) -> f64 {
        self.completed() as f64 / self.elapsed_seconds
    }

    fn successful_qps(&self) -> f64 {
        self.stats.succeeded as f64 / self.elapsed_seconds
    }

    fn throughput_mib(&self) -> f64 {
        self.total_bytes() as f64 / self.elapsed_seconds / 1024.0 / 1024.0
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let concurrencies = env_usize_list("PT_CONCURRENCY", DEFAULT_CONCURRENCIES)?;
    let requests_per_task = env_usize("PT_REQUESTS_PER_TASK", DEFAULT_REQUESTS_PER_TASK)?;
    let rounds = env_usize("PT_ROUNDS", DEFAULT_ROUNDS)?;
    let server_url = env::var("PT_SERVER_URL").unwrap_or_else(|_| DEFAULT_SERVER_URL.to_owned());
    let request_body_bytes = serde_json::to_vec(&RequestDto {
        str: REQUEST_VALUE.to_owned(),
    })?
    .len() as u64;
    let protocols = benchmark_protocols()?;
    let mut summaries = Vec::with_capacity(protocols.len() * concurrencies.len());
    let mut failures = 0_u64;
    for concurrency in concurrencies {
        let planned_requests = concurrency
            .checked_mul(requests_per_task)
            .ok_or_else(|| invalid_input("压测请求总数溢出"))?;
        let config = BenchmarkConfig {
            concurrency,
            requests_per_task,
            planned_requests,
            server_url: server_url.clone(),
            request_body_bytes,
        };
        for protocol in protocols.iter().copied() {
            let runtime = ClientRuntime::builder().build()?;
            let client = DemoServiceClient::builder(&runtime)
                .direct(&config.server_url)
                .protocol(protocol.wire_protocol())
                .connect()
                .await?;

            // Warm each protocol at benchmark concurrency before timing steady-state RPCs.
            warm_up(&client, config.concurrency).await?;

            let mut results = Vec::with_capacity(rounds);
            for round in 1..=rounds {
                let result = run_benchmark(&client, protocol, &config).await?;
                failures = failures.saturating_add(result.stats.failed);
                print_result(&config, &result, round, rounds);
                results.push(result);
            }
            runtime.shutdown().await?;
            let summary = summarize(&results, concurrency);
            print_summary(&summary, rounds);
            summaries.push(summary);
        }
    }

    print_comparisons(&summaries);
    if failures > 0 {
        return Err(invalid_input(format!("{failures} 个压测请求失败")).into());
    }
    Ok(())
}

async fn warm_up(
    client: &DemoServiceClient,
    concurrency: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut tasks = JoinSet::new();
    for _ in 0..concurrency {
        let client = client.clone();
        tasks.spawn(async move {
            client
                .say_hello_v2(RequestDto {
                    str: REQUEST_VALUE.to_owned(),
                })
                .await
                .map(|_| ())
        });
    }
    while let Some(result) = tasks.join_next().await {
        result??;
    }
    Ok(())
}

async fn run_benchmark(
    client: &DemoServiceClient,
    protocol: BenchmarkProtocol,
    config: &BenchmarkConfig,
) -> Result<BenchmarkResult, tokio::task::JoinError> {
    let started = Instant::now();
    let mut tasks = JoinSet::new();
    for _ in 0..config.concurrency {
        let client = client.clone();
        let requests_per_task = config.requests_per_task;
        tasks.spawn(async move {
            let mut stats = TaskStats::default();
            for _ in 0..requests_per_task {
                match client
                    .say_hello_v2(RequestDto {
                        str: REQUEST_VALUE.to_owned(),
                    })
                    .await
                {
                    Ok(response) => {
                        stats.succeeded += 1;
                        stats.response_body_bytes += serde_json::to_vec(&response)
                            .expect("ResponseDto serialization must succeed")
                            .len() as u64;
                    }
                    Err(error) => {
                        stats.failed += 1;
                        if stats.first_error.is_none() {
                            stats.first_error = Some(error.to_string());
                        }
                    }
                }
            }
            stats
        });
    }

    let mut stats = TaskStats::default();
    while let Some(result) = tasks.join_next().await {
        let task = result?;
        stats.succeeded += task.succeeded;
        stats.failed += task.failed;
        stats.response_body_bytes += task.response_body_bytes;
        if stats.first_error.is_none() {
            stats.first_error = task.first_error;
        }
    }
    Ok(BenchmarkResult {
        protocol,
        stats,
        elapsed_seconds: started.elapsed().as_secs_f64(),
        request_body_bytes: config.request_body_bytes,
    })
}

fn print_result(config: &BenchmarkConfig, result: &BenchmarkResult, round: usize, rounds: usize) {
    let request_bytes = result.request_bytes();
    let total_bytes = result.total_bytes();
    let average_response_bytes = if result.stats.succeeded == 0 {
        0.0
    } else {
        result.stats.response_body_bytes as f64 / result.stats.succeeded as f64
    };

    println!(
        "\n{} 压测结果，第 {round}/{rounds} 轮（不包含并发预热请求）",
        result.protocol.label(),
    );
    println!("  服务地址:             {}", config.server_url);
    println!("  并发任务数:           {}", config.concurrency);
    println!("  每任务请求数:         {}", config.requests_per_task);
    println!("  计划请求数:           {}", config.planned_requests);
    println!("  完成请求数:           {}", result.completed());
    println!(
        "  成功 / 失败:          {} / {}",
        result.stats.succeeded, result.stats.failed
    );
    println!("  总耗时:               {:.3} s", result.elapsed_seconds);
    println!("  总 QPS:               {:.2}", result.qps());
    println!("  成功 QPS:             {:.2}", result.successful_qps());
    println!(
        "  请求 JSON body:      {} ({request_bytes} B, {} B/次)",
        format_bytes(request_bytes),
        result.request_body_bytes,
    );
    println!(
        "  响应 JSON body:      {} ({} B, {average_response_bytes:.2} B/成功请求)",
        format_bytes(result.stats.response_body_bytes),
        result.stats.response_body_bytes
    );
    println!(
        "  JSON body 总传输量:  {} ({total_bytes} B)",
        format_bytes(total_bytes)
    );
    println!(
        "  JSON body 吞吐率:    {:.2} MiB/s",
        result.throughput_mib()
    );
    println!("  注: 字节统计不包含 HTTP 帧头或 TCP/IP 开销");

    if let Some(error) = result.stats.first_error.as_deref() {
        println!("  首个请求错误:         {error}");
    }
}

fn summarize(results: &[BenchmarkResult], concurrency: usize) -> BenchmarkSummary {
    let protocol = results
        .first()
        .expect("at least one benchmark round is required")
        .protocol;
    BenchmarkSummary {
        protocol,
        concurrency,
        median_qps: median(results.iter().map(BenchmarkResult::qps)),
        median_successful_qps: median(results.iter().map(BenchmarkResult::successful_qps)),
        median_throughput_mib: median(results.iter().map(BenchmarkResult::throughput_mib)),
    }
}

fn print_summary(summary: &BenchmarkSummary, rounds: usize) {
    println!(
        "\n{} 汇总（并发 {}，{rounds} 轮中位数）",
        summary.protocol.label(),
        summary.concurrency
    );
    println!("  总 QPS:               {:.2}", summary.median_qps);
    println!(
        "  成功 QPS:             {:.2}",
        summary.median_successful_qps
    );
    println!(
        "  JSON body 吞吐率:    {:.2} MiB/s",
        summary.median_throughput_mib
    );
}

fn print_comparisons(summaries: &[BenchmarkSummary]) {
    let mut concurrencies = summaries
        .iter()
        .map(|summary| summary.concurrency)
        .collect::<Vec<_>>();
    concurrencies.sort_unstable();
    concurrencies.dedup();
    for concurrency in concurrencies {
        let http1 = summaries.iter().find(|summary| {
            summary.concurrency == concurrency && summary.protocol == BenchmarkProtocol::Http1
        });
        let http2 = summaries.iter().find(|summary| {
            summary.concurrency == concurrency && summary.protocol == BenchmarkProtocol::Http2
        });
        let (Some(http1), Some(http2)) = (http1, http2) else {
            continue;
        };
        print_comparison(http1, http2);
    }
}

fn print_comparison(http1: &BenchmarkSummary, http2: &BenchmarkSummary) {
    let qps_ratio = if http1.median_successful_qps == 0.0 {
        0.0
    } else {
        http2.median_successful_qps / http1.median_successful_qps
    };
    let throughput_ratio = if http1.median_throughput_mib == 0.0 {
        0.0
    } else {
        http2.median_throughput_mib / http1.median_throughput_mib
    };

    println!(
        "\nHTTP/1.1 与 HTTP/2 对比（并发 {}，相同负载、顺序执行）",
        http1.concurrency
    );
    println!("  HTTP/1.1 成功 QPS:    {:.2}", http1.median_successful_qps);
    println!("  HTTP/2 成功 QPS:      {:.2}", http2.median_successful_qps);
    println!("  HTTP/2 / HTTP/1.1:    {qps_ratio:.3}x QPS");
    println!(
        "  HTTP/1.1 JSON 吞吐:  {:.2} MiB/s",
        http1.median_throughput_mib
    );
    println!(
        "  HTTP/2 JSON 吞吐:    {:.2} MiB/s",
        http2.median_throughput_mib
    );
    println!("  HTTP/2 / HTTP/1.1:    {throughput_ratio:.3}x JSON 吞吐");
    println!("  注: 使用各轮中位数，协议仍为顺序执行，应关注系统噪声");
}

fn median(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn benchmark_protocols() -> Result<Vec<BenchmarkProtocol>, io::Error> {
    let value = env::var("PT_PROTOCOL").unwrap_or_else(|_| "h2".to_owned());
    match value.to_ascii_lowercase().as_str() {
        "h1" | "http1" | "http/1.1" => Ok(vec![BenchmarkProtocol::Http1]),
        "h2" | "http2" | "http/2" => Ok(vec![BenchmarkProtocol::Http2]),
        "both" => Ok(vec![BenchmarkProtocol::Http1, BenchmarkProtocol::Http2]),
        _ => Err(invalid_input(
            "PT_PROTOCOL 仅支持 h1、h2 或 both（默认 h2）",
        )),
    }
}

fn env_usize(name: &str, default: usize) -> Result<usize, io::Error> {
    let value = match env::var(name) {
        Ok(value) => value
            .parse::<usize>()
            .map_err(|error| invalid_input(format!("{name} 必须是正整数: {error}")))?,
        Err(env::VarError::NotPresent) => default,
        Err(error) => return Err(invalid_input(format!("无法读取 {name}: {error}"))),
    };
    if value == 0 {
        return Err(invalid_input(format!("{name} 必须大于 0")));
    }
    Ok(value)
}

fn env_usize_list(name: &str, default: &[usize]) -> Result<Vec<usize>, io::Error> {
    let mut values = match env::var(name) {
        Ok(value) => value
            .split(',')
            .map(str::trim)
            .map(|value| {
                value.parse::<usize>().map_err(|error| {
                    invalid_input(format!("{name} 必须是逗号分隔的正整数: {error}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Err(env::VarError::NotPresent) => default.to_vec(),
        Err(error) => return Err(invalid_input(format!("无法读取 {name}: {error}"))),
    };
    if values.is_empty() || values.contains(&0) {
        return Err(invalid_input(format!("{name} 中的值必须大于 0")));
    }
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes / KIB)
    } else {
        format!("{bytes:.0} B")
    }
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
