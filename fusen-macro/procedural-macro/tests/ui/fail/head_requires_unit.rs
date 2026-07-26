use fusen_procedural_macro::service;

#[service(name = "health")]
trait HealthService {
    #[fusen_procedural_macro::method(
        idempotency = "safe",
        spring(method = "HEAD", path = "/health")
    )]
    async fn health(&self) -> Result<String, RpcError>;
}

struct RpcError;

fn main() {}
