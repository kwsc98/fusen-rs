# fusen-observability

`fusen-observability` defines the backend-neutral metrics contract used by the
Fusen runtimes. The default feature set exports only synchronous,
low-cardinality types and has no subscriber or exporter side effects.

Implement `MetricsRecorder` to receive borrowed `MetricEvent` values:

```rust
use fusen_observability::{MetricEvent, MetricsRecorder};

struct Recorder;

impl MetricsRecorder for Recorder {
    fn record(&self, event: &MetricEvent<'_>) {
        // Forward to an application-owned, non-blocking metrics backend.
        let _ = event;
    }
}
```

Recorders must be synchronous and non-blocking. `fusen-rs` isolates recorder
panics and disables a recorder after its first panic so metrics cannot break a
service invocation or runtime lifecycle.

Events cover logical invocations, transport attempts, admission rejection,
registry operations, directory and circuit state changes, and shutdown. They
intentionally exclude request IDs, endpoint addresses, bodies, credentials,
full headers, error text, and other unbounded values from labels.

The core runtime emits structured `tracing` spans and events independently.
Applications own subscriber/exporter initialization and must retain any backend
guard themselves. The optional `otel` feature exposes
`otel::OpenTelemetryMetricsRecorder`, which builds instruments from an
application-owned `Meter` and never installs a global provider or exporter.

Version 0.9 defines the first compatibility baseline. Requires Rust 1.97 or
newer. Licensed under Apache-2.0.
