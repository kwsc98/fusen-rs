use fusen_procedural_macro::service;

#[service(name = "user")]
trait UserService {
    #[fusen_procedural_macro::method(
        idempotency = "safe",
        spring(method = "POST", path = "/users")
    )]
    async fn get(&self) -> Result<(), RpcError>;
}

struct RpcError;

fn main() {}
