use bytes::Bytes;
use fusen_contract::{
    MethodDescriptor, MethodId, ServiceDescriptor, ServiceEndpoint, WireProtocol,
};
use fusen_rs::{
    __private::{__benchmark_middleware, FusenHttpCodec, FusenRequest, Path, RequestCodec},
    InvocationFinish, InvocationObserver, InvocationOutcome, InvocationPhase, InvocationSide,
    InvocationStart,
};
use http::{Method, Request, StatusCode, header::CONTENT_TYPE};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use std::{
    convert::Infallible,
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

const CHAIN_ITERATIONS: u64 = 100_000;
const DISPATCH_ITERATIONS: u64 = 1_000_000;
const CODEC_SMALL_ITERATIONS: u64 = 10_000;
const CODEC_LARGE_ITERATIONS: u64 = 1_000;

struct NoopObserver;

impl InvocationObserver for NoopObserver {
    fn on_start(&self, _event: &InvocationStart<'_>) {}

    fn on_finish(&self, _event: &InvocationFinish<'_>) {}
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("benchmark runtime must build");
    println!("fusen-rs invocation microbenchmarks");
    println!("lower ns/op is better; run on an otherwise idle machine");

    for middleware_count in [0, 1, 4, 8] {
        benchmark_chain(&runtime, middleware_count);
    }
    benchmark_observers(0);
    benchmark_observers(1);
    benchmark_method_dispatch();
    benchmark_codec_encode(1_024, CODEC_SMALL_ITERATIONS);
    benchmark_codec_decode(&runtime, 1_024, CODEC_SMALL_ITERATIONS);
    benchmark_codec_encode(64 * 1_024, CODEC_LARGE_ITERATIONS);
    benchmark_codec_decode(&runtime, 64 * 1_024, CODEC_LARGE_ITERATIONS);
}

fn benchmark_chain(runtime: &Runtime, middleware_count: usize) {
    let elapsed = runtime.block_on(__benchmark_middleware(middleware_count, CHAIN_ITERATIONS));
    report(
        &format!("chain/{middleware_count}-middleware"),
        CHAIN_ITERATIONS,
        elapsed,
    );
}

fn benchmark_observers(observer_count: usize) {
    let observers = (0..observer_count)
        .map(|_| Arc::new(NoopObserver) as Arc<dyn InvocationObserver>)
        .collect::<Vec<_>>();
    let start = InvocationStart {
        side: InvocationSide::Client,
        request_id: "benchmark-request",
        service: Some("benchmark-service"),
        method: Some("call"),
    };
    let finish = InvocationFinish {
        side: InvocationSide::Client,
        request_id: "benchmark-request",
        service: Some("benchmark-service"),
        method: Some("call"),
        phase: InvocationPhase::Complete,
        outcome: InvocationOutcome::Success,
        elapsed: Duration::from_micros(1),
        http_status: Some(StatusCode::OK),
        error_code: None,
    };
    let started = Instant::now();
    for _ in 0..DISPATCH_ITERATIONS {
        for observer in &observers {
            observer.on_start(black_box(&start));
            observer.on_finish(black_box(&finish));
        }
    }
    report(
        &format!("observer/{observer_count}"),
        DISPATCH_ITERATIONS,
        started.elapsed(),
    );
}

fn benchmark_method_dispatch() {
    let descriptor = ServiceDescriptor::__new(
        "benchmark-service",
        None,
        None,
        (0..32).map(method_info).collect(),
    )
    .expect("benchmark descriptor must be valid");
    let started = Instant::now();
    for _ in 0..DISPATCH_ITERATIONS {
        black_box(descriptor.method(MethodId::__new(17))).expect("indexed method must exist");
    }
    report(
        "client-dispatch/index",
        DISPATCH_ITERATIONS,
        started.elapsed(),
    );
}

fn benchmark_codec_encode(payload_bytes: usize, iterations: u64) {
    let codec = FusenHttpCodec::default();
    let payload = "x".repeat(payload_bytes.saturating_sub(2));
    let endpoint = "http://127.0.0.1:8081"
        .parse::<ServiceEndpoint>()
        .expect("benchmark endpoint must be valid");
    let started = Instant::now();
    for _ in 0..iterations {
        let mut request = codec_request(endpoint.clone(), payload.clone());
        black_box(RequestCodec::encode(&codec, &mut request))
            .expect("benchmark request must encode");
    }
    report(
        &format!("codec/encode-{payload_bytes}-bytes"),
        iterations,
        started.elapsed(),
    );
}

fn benchmark_codec_decode(runtime: &Runtime, payload_bytes: usize, iterations: u64) {
    let codec = FusenHttpCodec::default();
    let json = Bytes::from(format!(
        "\"{}\"",
        "x".repeat(payload_bytes.saturating_sub(2))
    ));
    let elapsed = runtime.block_on(async {
        let started = Instant::now();
        for _ in 0..iterations {
            let body: BoxBody<Bytes, hyper::Error> = Full::new(json.clone())
                .map_err(|never: Infallible| match never {})
                .boxed();
            let request = Request::builder()
                .method(Method::POST)
                .uri("/benchmark")
                .header(CONTENT_TYPE, "application/json")
                .body(body)
                .expect("benchmark HTTP request must build");
            black_box(RequestCodec::decode(&codec, request).await)
                .expect("benchmark request must decode");
        }
        started.elapsed()
    });
    report(
        &format!("codec/decode-{payload_bytes}-bytes"),
        iterations,
        elapsed,
    );
}

fn method_info(index: u16) -> MethodDescriptor {
    MethodDescriptor::__new(
        MethodId::__new(index),
        format!("method-{index}"),
        Method::POST,
        format!("/benchmark/{index}"),
        Vec::new(),
    )
    .expect("benchmark method must be valid")
}

fn codec_request(endpoint: ServiceEndpoint, payload: String) -> FusenRequest {
    FusenRequest {
        protocol: WireProtocol::Fusen,
        path: Path {
            method: Method::POST,
            path: "/benchmark".to_owned(),
        },
        endpoint: Some(endpoint),
        path_parameters: Default::default(),
        query_parameters: Default::default(),
        headers: Default::default(),
        body: Some(serde_json::Value::String(payload)),
    }
}

fn report(label: &str, iterations: u64, elapsed: Duration) {
    let ns_per_iteration = elapsed.as_nanos() as f64 / iterations as f64;
    let operations_per_second = iterations as f64 / elapsed.as_secs_f64();
    println!("{label:<36} {ns_per_iteration:>12.2} ns/op {operations_per_second:>14.0} ops/s");
}
