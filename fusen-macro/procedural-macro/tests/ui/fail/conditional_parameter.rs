use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "conditional-parameter")]
trait ConditionalParameter {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(&self, #[cfg(any())] hidden: String) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
