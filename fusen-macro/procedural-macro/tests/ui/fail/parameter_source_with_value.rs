use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "parameter-source-with-value")]
trait ParameterSourceWithValue {
    #[fusen_procedural_macro::method(method = "GET", path = "/items")]
    async fn items(
        &self,
        #[param(query = true)] filter: String,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
