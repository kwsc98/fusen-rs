use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "missing-role")]
trait MissingRole {
    async fn call(&self, id: String) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
