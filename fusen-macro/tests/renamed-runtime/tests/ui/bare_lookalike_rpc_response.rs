use rpc::interface;

struct RpcResponse<T>(T);

#[interface(name = "bare-lookalike-response")]
trait BareLookalikeResponse {
    #[rpc::method(method = "GET", path = "/response")]
    async fn call(&self) -> Result<RpcResponse<String>, rpc::RpcError>;
}

fn main() {}
