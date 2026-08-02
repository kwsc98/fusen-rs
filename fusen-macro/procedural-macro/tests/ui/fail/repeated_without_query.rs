use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "repeated-without-query")]
trait RepeatedWithoutQuery {
    #[fusen_procedural_macro::method(method = "GET", path = "/tags")]
    async fn tags(
        &self,
        #[param(repeated)] tags: Vec<String>,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
