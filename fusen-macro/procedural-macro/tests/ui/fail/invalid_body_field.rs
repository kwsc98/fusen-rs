use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "repeated-body-field")]
trait RepeatedBodyField {
    #[fusen_procedural_macro::method(method = "POST", path = "/repeated")]
    async fn call(
        &self,
        #[param(body_field, repeated)] values: Vec<String>,
    ) -> Result<Response<()>, Error>;
}

#[interface(name = "valued-body-field")]
trait ValuedBodyField {
    #[fusen_procedural_macro::method(method = "POST", path = "/valued")]
    async fn call(
        &self,
        #[param(body_field = true)] value: String,
    ) -> Result<Response<()>, Error>;
}

#[interface(name = "mixed-explicit-body")]
trait MixedExplicitBody {
    #[fusen_procedural_macro::method(method = "POST", path = "/mixed")]
    async fn call(
        &self,
        #[param(body_field)] field: String,
        #[param(body)] document: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
