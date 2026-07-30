use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "invalid-call")]
trait InvalidCallType {
    async fn call(
        &self,
        #[rpc(call)] call: String,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
