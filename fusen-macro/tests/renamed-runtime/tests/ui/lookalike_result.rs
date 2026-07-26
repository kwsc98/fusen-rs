use rpc::service;

mod lookalike {
    pub struct RpcError;

    pub enum Result<T, E> {
        Ok(T),
        Err(E),
    }
}

#[service(name = "user")]
trait UserService {
    async fn get(&self) -> lookalike::Result<(), lookalike::RpcError>;
}

fn main() {}
