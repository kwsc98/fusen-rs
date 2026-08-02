use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "invalid-call")]
trait InvalidCallType {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(
        &self,
        #[param(context)] call: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
