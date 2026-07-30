use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "optional-query")]
trait OptionalRepeatedQuery {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(
        &self,
        #[param(query)] tags: Option<Vec<String>>,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
