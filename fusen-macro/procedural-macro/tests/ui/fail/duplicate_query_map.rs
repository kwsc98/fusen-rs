use fusen_procedural_macro::interface;

struct Response<T>(T);
struct Error;

#[interface(name = "duplicate-query-map")]
trait DuplicateQueryMap {
    #[fusen_procedural_macro::method(method = "GET", path = "/items")]
    async fn list(
        &self,
        #[param(query_map)] first: String,
        #[param(query_map)] second: String,
    ) -> Result<Response<String>, Error>;
}

fn main() {}
