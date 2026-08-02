use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "generic-method")]
trait GenericMethod {
    #[fusen_procedural_macro::method(method = "POST", path = "/value")]
    async fn value<T>(&self, value: T) -> Result<Response<()>, Error>;
}

fn main() {}
