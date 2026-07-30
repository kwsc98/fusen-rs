use fusen_procedural_macro::interface;

struct RpcError;
struct RpcResponse<T>(T);

#[interface(name = "removed-idempotency")]
pub trait RemovedIdempotencyApi {
    #[fusen_procedural_macro::method(idempotency = "safe")]
    async fn get(&self) -> Result<RpcResponse<String>, RpcError>;
}

fn main() {}
