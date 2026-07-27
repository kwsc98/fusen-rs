use rpc::service;

mod lookalike {
    pub struct RpcError;

    pub enum LookalikeResult<T, E> {
        Ok(T),
        Err(E),
    }

    pub type Result<T, E> = LookalikeResult<T, E>;
}

#[service(name = "user")]
trait UserService {
    async fn get(&self) -> lookalike::Result<(), lookalike::RpcError>;
}

fn main() {}
