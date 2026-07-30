use rpc::interface;

mod lookalike {
    pub struct RpcError;

    pub enum LookalikeResult<T, E> {
        Ok(T),
        Err(E),
    }

    pub type Result<T, E> = LookalikeResult<T, E>;
}

#[interface(name = "user")]
trait UserService {
    #[rpc::method(method = "GET", path = "/users")]
    async fn get(&self) -> lookalike::Result<rpc::RpcResponse<()>, rpc::RpcError>;
}

fn main() {}
