use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "optional-query")]
trait OptionalRepeatedQuery {
    async fn call(
        &self,
        #[rpc(query)] tags: Option<Vec<String>>,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
