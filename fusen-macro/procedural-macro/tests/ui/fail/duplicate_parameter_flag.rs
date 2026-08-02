use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "duplicate-parameter-flag")]
trait DuplicateParameterFlag {
    #[fusen_procedural_macro::method(method = "GET", path = "/tags")]
    async fn tags(
        &self,
        #[param(query, repeated, repeated)] tags: Vec<String>,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
