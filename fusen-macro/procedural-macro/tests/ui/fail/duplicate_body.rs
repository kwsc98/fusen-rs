use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "duplicate-body")]
trait DuplicateBody {
    #[fusen_procedural_macro::method(method = "POST", path = "/duplicate")]
    async fn call(
        &self,
        #[param(body)] first: String,
        #[param(body)] second: String,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
