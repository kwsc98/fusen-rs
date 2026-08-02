use fusen_procedural_macro::interface;

struct Response<T>(T);
struct Error;

#[interface(name = "duplicate-header-map")]
trait DuplicateHeaderMap {
    #[fusen_procedural_macro::method(method = "GET", path = "/items")]
    async fn list(
        &self,
        #[param(header_map)] first: String,
        #[param(header_map)] second: String,
    ) -> Result<Response<String>, Error>;
}

fn main() {}
