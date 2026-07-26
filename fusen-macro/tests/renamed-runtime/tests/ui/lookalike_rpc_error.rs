use rpc::service;

mod lookalike {
    pub struct RpcError;
}

#[service(name = "user")]
trait UserService {
    async fn get(&self) -> Result<(), lookalike::RpcError>;
}

fn main() {}
