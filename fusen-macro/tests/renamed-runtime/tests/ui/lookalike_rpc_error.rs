use rpc::interface;

mod lookalike {
    pub struct RpcError;
}

#[interface(name = "user")]
trait UserService {
    #[rpc::method(method = "GET", path = "/users")]
    async fn get(&self) -> Result<rpc::RpcResponse<()>, lookalike::RpcError>;
}

fn main() {}
