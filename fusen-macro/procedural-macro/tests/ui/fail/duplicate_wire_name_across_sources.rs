use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "duplicate-wire-name")]
trait DuplicateWireNameAcrossSources {
    #[fusen_procedural_macro::method(method = "GET", path = "/users/{user_id}")]
    async fn get(
        &self,
        #[param(path, name = "user_id")] path_user_id: String,
        #[param(query, name = "user_id")] query_user_id: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
