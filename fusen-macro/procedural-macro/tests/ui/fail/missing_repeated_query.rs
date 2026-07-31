use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "missing-repeated-query")]
trait MissingRepeatedQuery {
    #[fusen_procedural_macro::method(method = "GET", path = "/tags")]
    async fn tags(&self, #[param(query)] tags: Vec<String>) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
