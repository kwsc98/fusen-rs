use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "path-mismatch")]
trait PathMismatch {
    #[fusen_procedural_macro::method(
        idempotency = "safe",
        spring(method = "GET", path = "/users/{id}")
    )]
    async fn get(
        &self,
        #[rpc(query)] id: String,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
