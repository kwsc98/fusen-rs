use fusen_procedural_macro::interface;

struct Response<T>(T);
struct Error;

#[interface(name = "repeated-parameter-map")]
trait RepeatedParameterMap {
    #[fusen_procedural_macro::method(method = "GET", path = "/items")]
    async fn list(
        &self,
        #[param(header_map, repeated)] headers: Vec<String>,
    ) -> Result<Response<String>, Error>;
}

fn main() {}
