use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "repeated-without-query")]
trait RepeatedWithoutQuery {
    #[fusen_procedural_macro::method(method = "GET", path = "/tags")]
    async fn tags(
        &self,
        #[param(repeated)] tags: Vec<String>,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
