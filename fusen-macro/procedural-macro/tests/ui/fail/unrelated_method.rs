use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "unrelated-method")]
trait UnrelatedMethod {
    #[unrelated::method(method = "GET", path = "/call")]
    async fn call(&self) -> Result<Response<()>, Error>;
}

fn main() {}
