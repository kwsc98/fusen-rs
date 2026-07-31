use rpc::interface;

#[derive(serde::Serialize, serde::Deserialize)]
struct Request {
    value: String,
}

#[interface(name = "missing-request-sensitivity")]
trait MissingRequestSensitivity {
    #[rpc::method(method = "POST", path = "/request")]
    async fn call(
        &self,
        #[param(body)] request: Request,
    ) -> Result<rpc::RpcResponse<String>, rpc::RpcError>;
}

fn main() {}
