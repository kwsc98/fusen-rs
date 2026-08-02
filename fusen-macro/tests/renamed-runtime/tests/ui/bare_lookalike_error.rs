use runtime::interface;

struct Error;

#[interface(name = "bare-lookalike-error")]
trait BareLookalikeError {
    #[runtime::method(method = "GET", path = "/error")]
    async fn call(&self) -> Result<runtime::Response<String>, Error>;
}

fn main() {}
