use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "noncanonical-route")]
trait NoncanonicalRoute {
    #[fusen_procedural_macro::method(method = "GET", path = "/用户")]
    async fn call(&self) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
