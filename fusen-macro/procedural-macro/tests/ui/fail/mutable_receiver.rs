use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "mutable-receiver")]
trait MutableReceiver {
    #[fusen_procedural_macro::method(method = "GET", path = "/value")]
    async fn value(&mut self) -> Result<Response<()>, Error>;
}

fn main() {}
