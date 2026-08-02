use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "missing-method")]
trait MissingMethod {
    async fn call(&self, id: String) -> Result<Response<()>, Error>;
}

fn main() {}
