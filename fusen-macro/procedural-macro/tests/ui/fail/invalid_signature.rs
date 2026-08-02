use fusen_procedural_macro::interface;

struct Input;
struct Output;
struct Error;
struct Response<T>(T);

#[interface(name = "invalid")]
trait InvalidSignature {
    #[fusen_procedural_macro::method(method = "POST", path = "/call")]
    async fn call(&self, #[param(body)] request: Input) -> Result<Output, Error>;
}

fn main() {}
