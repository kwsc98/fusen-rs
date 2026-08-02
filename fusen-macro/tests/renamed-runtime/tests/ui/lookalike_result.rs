use runtime::interface;

mod lookalike {
    pub struct Error;

    pub enum LookalikeResult<T, E> {
        Ok(T),
        Err(E),
    }

    pub type Result<T, E> = LookalikeResult<T, E>;
}

#[interface(name = "user")]
trait UserService {
    #[runtime::method(method = "GET", path = "/users")]
    async fn get(&self) -> lookalike::Result<runtime::Response<()>, runtime::Error>;
}

fn main() {}
