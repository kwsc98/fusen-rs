use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "duplicate-parameter-flag")]
trait DuplicateParameterFlag {
    #[fusen_procedural_macro::method(method = "GET", path = "/tags")]
    async fn tags(
        &self,
        #[param(query, repeated, repeated)] tags: Vec<String>,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
