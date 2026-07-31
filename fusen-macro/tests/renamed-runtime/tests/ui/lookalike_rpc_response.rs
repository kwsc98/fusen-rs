use rpc::interface;

mod lookalike {
    pub struct RpcResponse<T>(pub T);
}

#[interface(name = "lookalike-response")]
trait LookalikeResponse {
    #[rpc::method(method = "GET", path = "/response")]
    async fn call(&self) -> Result<lookalike::RpcResponse<String>, rpc::RpcError>;
}

fn main() {}
