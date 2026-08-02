use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "path-without-placeholder")]
trait PathWithoutPlaceholder {
    #[fusen_procedural_macro::method(method = "GET", path = "/users")]
    async fn get(
        &self,
        #[param(path)] user_id: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
