use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "unrelated-method")]
trait UnrelatedMethod {
    #[unrelated::method(method = "GET", path = "/call")]
    async fn call(&self) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
