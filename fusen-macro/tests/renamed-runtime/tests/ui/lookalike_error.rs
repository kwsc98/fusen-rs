use runtime::interface;

mod lookalike {
    pub struct Error;
}

#[interface(name = "user")]
trait UserService {
    #[runtime::method(method = "GET", path = "/users")]
    async fn get(&self) -> Result<runtime::Response<()>, lookalike::Error>;
}

fn main() {}
