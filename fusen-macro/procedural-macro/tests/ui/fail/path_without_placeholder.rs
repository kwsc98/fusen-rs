use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "path-without-placeholder")]
trait PathWithoutPlaceholder {
    #[fusen_procedural_macro::method(method = "GET", path = "/users")]
    async fn get(
        &self,
        #[param(path)] user_id: String,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
