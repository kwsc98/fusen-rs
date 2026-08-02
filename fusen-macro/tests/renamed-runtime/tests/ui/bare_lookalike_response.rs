use runtime::interface;

struct Response<T>(T);

#[interface(name = "bare-lookalike-response")]
trait BareLookalikeResponse {
    #[runtime::method(method = "GET", path = "/response")]
    async fn call(&self) -> Result<Response<String>, runtime::Error>;
}

fn main() {}
