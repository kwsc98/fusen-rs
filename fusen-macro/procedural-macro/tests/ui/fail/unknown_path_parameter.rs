use fusen_procedural_macro::service;

#[service(name = "user")]
trait UserService {
    #[fusen_procedural_macro::method(spring(method = "GET", path = "/users/{id}"))]
    async fn get(&self, name: String) -> Result<(), RpcError>;
}

struct RpcError;

fn main() {}
