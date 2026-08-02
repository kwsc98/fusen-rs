use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "noncanonical-route")]
trait NoncanonicalRoute {
    #[fusen_procedural_macro::method(method = "GET", path = "/用户")]
    async fn call(&self) -> Result<Response<()>, Error>;
}

fn main() {}
