use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "repeated-non-query")]
trait RepeatedNonQuery {
    #[fusen_procedural_macro::method(method = "POST", path = "/tags")]
    async fn tags(
        &self,
        #[param(body, repeated)] tags: Vec<String>,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
