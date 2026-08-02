use runtime::interface;

mod lookalike {
    pub struct Response<T>(pub T);
}

#[interface(name = "lookalike-response")]
trait LookalikeResponse {
    #[runtime::method(method = "GET", path = "/response")]
    async fn call(&self) -> Result<lookalike::Response<String>, runtime::Error>;
}

fn main() {}
