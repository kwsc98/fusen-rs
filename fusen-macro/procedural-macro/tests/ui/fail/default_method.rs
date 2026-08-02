use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "default-method")]
trait DefaultMethod {
    #[fusen_procedural_macro::method(method = "GET", path = "/value")]
    async fn value(&self) -> Result<Response<()>, Error> {
        unreachable!()
    }
}

fn main() {}
