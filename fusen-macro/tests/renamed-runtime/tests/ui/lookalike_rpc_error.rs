use rpc::interface;

mod lookalike {
    pub struct RpcError;
}

#[interface(name = "user")]
trait UserService {
    async fn get(&self) -> Result<rpc::RpcResponse<()>, lookalike::RpcError>;
}

fn main() {}
