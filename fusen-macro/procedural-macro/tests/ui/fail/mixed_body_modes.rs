use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "mixed-body-modes")]
trait MixedBodyModes {
    #[fusen_procedural_macro::method(method = "POST", path = "/users")]
    async fn create(
        &self,
        #[param(body)] document: String,
        audit: bool,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
