use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "invalid-path-type")]
trait InvalidPathType {
    #[fusen_procedural_macro::method(
        method = "GET", path = "/users/{id}"
    )]
    async fn get(
        &self,
        id: Option<String>,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
