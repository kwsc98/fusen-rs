use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "conflicting-parameter-sources")]
trait ConflictingParameterSources {
    #[fusen_procedural_macro::method(method = "GET", path = "/users/{user_id}")]
    async fn get(
        &self,
        #[param(path, query)] user_id: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
