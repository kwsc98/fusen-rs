use rpc::interface;

#[derive(serde::Serialize, serde::Deserialize)]
struct Response {
    value: String,
}

#[interface(name = "missing-response-sensitivity")]
trait MissingResponseSensitivity {
    #[rpc::method(method = "GET", path = "/response")]
    async fn call(&self) -> Result<rpc::RpcResponse<Response>, rpc::RpcError>;
}

fn main() {}
