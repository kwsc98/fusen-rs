use std::{env, io, time::Instant};

use examples::{DemoServiceClient, RequestDto};
use fusen_rs::{
    client::{ClientOptions, FusenClientContextBuilder},
    contract::WireProtocol,
};
use tokio::task::JoinSet;

const DEFAULT_CONCURRENCY: usize = 100;
const DEFAULT_REQUESTS_PER_TASK: usize = 10_000;
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
            Self::Http1 => WireProtocol::SpringCloud,
            Self::Http2 => WireProtocol::Fusen,
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
    let concurrency = env_usize("PT_CONCURRENCY", DEFAULT_CONCURRENCY)?;
    let requests_per_task = env_usize("PT_REQUESTS_PER_TASK", DEFAULT_REQUESTS_PER_TASK)?;
    let planned_requests = concurrency
        .checked_mul(requests_per_task)
        .ok_or_else(|| invalid_input("压测请求总数溢出"))?;
    let server_url = env::var("PT_SERVER_URL").unwrap_or_else(|_| DEFAULT_SERVER_URL.to_owned());
    let request_body_bytes = serde_json::to_vec(&RequestDto {
        str: REQUEST_VALUE.to_owned(),
    })?
    .len() as u64;
    let protocols = benchmark_protocols()?;
    let config = BenchmarkConfig {
        concurrency,
        requests_per_task,
        planned_requests,
        server_url,
        request_body_bytes,
    };

    let mut results = Vec::with_capacity(protocols.len());
    for protocol in protocols {
        let mut context = FusenClientContextBuilder::new().build()?;
        let mut options = ClientOptions::direct(config.server_url.parse()?);
        options.protocol = protocol.wire_protocol();
        let client = DemoServiceClient::init(&mut context, options).await?;

        // Warm each protocol at benchmark concurrency before timing steady-state RPCs.
        warm_up(&client, config.concurrency).await?;

        let result = run_benchmark(&client, protocol, &config).await?;
        client.close().await?;
        print_result(&config, &result);
        results.push(result);
    }

    print_comparison(&results);
    let failures = results
        .iter()
        .map(|result| result.stats.failed)
        .sum::<u64>();
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
                .sayHelloV2(RequestDto {
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
                    .sayHelloV2(RequestDto {
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

fn print_result(config: &BenchmarkConfig, result: &BenchmarkResult) {
    let request_bytes = result.request_bytes();
    let total_bytes = result.total_bytes();
    let average_response_bytes = if result.stats.succeeded == 0 {
        0.0
    } else {
        result.stats.response_body_bytes as f64 / result.stats.succeeded as f64
    };

    println!(
        "\n{} 压测结果（不包含并发预热请求）",
        result.protocol.label()
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
    println!("  注: 字节统计不包含 HTTP 帧头、TCP/IP 或 TLS 开销");

    if let Some(error) = result.stats.first_error.as_deref() {
        println!("  首个请求错误:         {error}");
    }
}

fn print_comparison(results: &[BenchmarkResult]) {
    let http1 = results
        .iter()
        .find(|result| result.protocol == BenchmarkProtocol::Http1);
    let http2 = results
        .iter()
        .find(|result| result.protocol == BenchmarkProtocol::Http2);
    let (Some(http1), Some(http2)) = (http1, http2) else {
        return;
    };
    let qps_ratio = if http1.successful_qps() == 0.0 {
        0.0
    } else {
        http2.successful_qps() / http1.successful_qps()
    };
    let throughput_ratio = if http1.throughput_mib() == 0.0 {
        0.0
    } else {
        http2.throughput_mib() / http1.throughput_mib()
    };

    println!("\nHTTP/1.1 与 HTTP/2 对比（相同负载、顺序执行）");
    println!("  HTTP/1.1 成功 QPS:    {:.2}", http1.successful_qps());
    println!("  HTTP/2 成功 QPS:      {:.2}", http2.successful_qps());
    println!("  HTTP/2 / HTTP/1.1:    {qps_ratio:.3}x QPS");
    println!("  HTTP/1.1 JSON 吞吐:  {:.2} MiB/s", http1.throughput_mib());
    println!("  HTTP/2 JSON 吞吐:    {:.2} MiB/s", http2.throughput_mib());
    println!("  HTTP/2 / HTTP/1.1:    {throughput_ratio:.3}x JSON 吞吐");
    println!("  注: 两轮顺序执行，正式比较应重复多轮并关注系统噪声");
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
