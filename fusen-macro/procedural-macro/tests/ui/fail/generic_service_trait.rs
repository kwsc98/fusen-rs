use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "generic-service")]
trait GenericService<T> {
    #[fusen_procedural_macro::method(method = "GET", path = "/value")]
    async fn value(&self) -> Result<Response<T>, Error>;
}

fn main() {}
