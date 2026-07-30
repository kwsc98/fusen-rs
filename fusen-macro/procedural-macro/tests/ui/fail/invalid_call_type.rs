use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "invalid-call")]
trait InvalidCallType {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(
        &self,
        #[param(context)] call: String,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
