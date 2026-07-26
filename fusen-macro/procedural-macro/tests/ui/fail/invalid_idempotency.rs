use fusen_procedural_macro::service;

#[service(name = "user")]
trait UserService {
    #[fusen_procedural_macro::method(idempotency = "automatic")]
    async fn get(&self) -> Result<(), RpcError>;
}

struct RpcError;

fn main() {}
