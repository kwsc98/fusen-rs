use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "missing-repeated-query")]
trait MissingRepeatedQuery {
    #[fusen_procedural_macro::method(method = "GET", path = "/tags")]
    async fn tags(&self, #[param(query)] tags: Vec<String>) -> Result<Response<()>, Error>;
}

fn main() {}
