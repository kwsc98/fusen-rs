use fusen_procedural_macro::service;

#[service(name = "user")]
trait UserService {
    #[fusen_procedural_macro::method(spring(method = "GET", path = "/users/{id}"))]
    async fn by_id(&self, id: String) -> Result<(), RpcError>;

    #[fusen_procedural_macro::method(spring(method = "get", path = "/users/{name}"))]
    async fn by_name(&self, name: String) -> Result<(), RpcError>;
}

struct RpcError;

fn main() {}
