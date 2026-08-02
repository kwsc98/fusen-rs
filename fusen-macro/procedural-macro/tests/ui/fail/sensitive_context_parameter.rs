use fusen_procedural_macro::interface;

struct Call;
struct Error;
struct Response<T>(T);

#[interface(name = "sensitive-context")]
trait SensitiveContext {
    #[fusen_procedural_macro::method(method = "GET", path = "/context")]
    async fn call(
        &self,
        #[param(context)]
        #[sensitive(kind = "identifier")]
        call: Call,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
