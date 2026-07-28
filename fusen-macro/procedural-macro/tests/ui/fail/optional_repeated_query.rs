use fusen_procedural_macro::service;

#[service(name = "search")]
trait SearchService {
    #[fusen_procedural_macro::method(
        spring(method = "GET", path = "/search", query = ["tags"])
    )]
    async fn search(&self, tags: Option<Vec<String>>) -> Result<(), RpcError>;
}

struct RpcError;

fn main() {}
