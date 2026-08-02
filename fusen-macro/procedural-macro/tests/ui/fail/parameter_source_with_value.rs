use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "parameter-source-with-value")]
trait ParameterSourceWithValue {
    #[fusen_procedural_macro::method(method = "GET", path = "/items")]
    async fn items(
        &self,
        #[param(query = true)] filter: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
