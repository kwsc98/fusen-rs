use fusen_procedural_macro::service;

#[service]
trait UserService {
    async fn get(&self) -> Result<(), RpcError>;
}

struct RpcError;

fn main() {}
