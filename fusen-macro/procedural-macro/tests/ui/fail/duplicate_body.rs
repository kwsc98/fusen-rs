use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "duplicate-body")]
trait DuplicateBody {
    #[fusen_procedural_macro::method(method = "POST", path = "/duplicate")]
    async fn call(
        &self,
        #[param(body)] first: String,
        #[param(body)] second: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
