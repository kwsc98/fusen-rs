use fusen_procedural_macro::interface;

struct RpcCall;
struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "sensitive-context")]
trait SensitiveContext {
    #[fusen_procedural_macro::method(method = "GET", path = "/context")]
    async fn call(
        &self,
        #[param(context)]
        #[sensitive(kind = "identifier")]
        call: RpcCall,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
