use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "head-non-unit")]
trait HeadNonUnit {
    #[fusen_procedural_macro::method(method = "HEAD", path = "/health")]
    async fn health(&self) -> Result<Response<String>, Error>;
}

fn main() {}
