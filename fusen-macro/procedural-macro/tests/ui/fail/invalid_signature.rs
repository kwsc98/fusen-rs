use fusen_procedural_macro::interface;

struct Input;
struct Output;
struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "invalid")]
trait InvalidSignature {
    async fn call(&self, #[rpc(body)] request: Input) -> Result<Output, RpcError>;
}

fn main() {}
