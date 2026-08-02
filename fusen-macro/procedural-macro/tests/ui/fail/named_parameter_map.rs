use fusen_procedural_macro::interface;

struct Response<T>(T);
struct Error;

#[interface(name = "named-parameter-map")]
trait NamedParameterMap {
    #[fusen_procedural_macro::method(method = "GET", path = "/items")]
    async fn list(
        &self,
        #[param(query_map, name = "query")] query: String,
    ) -> Result<Response<String>, Error>;
}

fn main() {}
