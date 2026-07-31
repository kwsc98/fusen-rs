use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "invalid-wire-name")]
trait InvalidWireName {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(
        &self,
        #[param(query, name = "")] value: String,
    ) -> Result<RpcResponse<()>, RpcError>;
}

fn main() {}
