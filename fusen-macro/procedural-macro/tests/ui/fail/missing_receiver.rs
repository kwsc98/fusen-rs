use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "missing-receiver")]
trait MissingReceiver {
    #[fusen_procedural_macro::method(method = "POST", path = "/value")]
    async fn value(value: String) -> Result<Response<()>, Error>;
}

fn main() {}
