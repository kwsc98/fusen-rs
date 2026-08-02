use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "removed-rpc-parameter")]
trait RemovedRpcParameter {
    #[fusen_procedural_macro::method(method = "GET", path = "/users/{id}")]
    async fn get(
        &self,
        #[rpc(path)] id: String,
    ) -> Result<Response<()>, Error>;
}

fn main() {}
