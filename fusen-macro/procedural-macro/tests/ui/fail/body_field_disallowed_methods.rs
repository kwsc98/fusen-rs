use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "get-body-field")]
trait GetBodyField {
    #[fusen_procedural_macro::method(method = "GET", path = "/get")]
    async fn call(
        &self,
        #[param(body_field)] value: String,
    ) -> Result<Response<()>, Error>;
}

#[interface(name = "head-body-field")]
trait HeadBodyField {
    #[fusen_procedural_macro::method(method = "HEAD", path = "/head")]
    async fn call(
        &self,
        #[param(body_field)] value: String,
    ) -> Result<Response<()>, Error>;
}

#[interface(name = "options-body-field")]
trait OptionsBodyField {
    #[fusen_procedural_macro::method(method = "OPTIONS", path = "/options")]
    async fn call(
        &self,
        #[param(body_field)] value: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
