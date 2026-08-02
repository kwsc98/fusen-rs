use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "path-mismatch")]
trait PathMismatch {
    #[fusen_procedural_macro::method(
        method = "GET", path = "/users/{id}"
    )]
    async fn get(
        &self,
        #[param(query)] id: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
