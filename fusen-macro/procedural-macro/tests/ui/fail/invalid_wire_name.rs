use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "invalid-wire-name")]
trait InvalidWireName {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(
        &self,
        #[param(query, name = "")] value: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
