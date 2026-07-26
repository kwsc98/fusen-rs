use fusen_procedural_macro::service;

#[service(name = "user")]
trait UserService {
    #[fusen_procedural_macro::method(spring(method = "GET", path = "/users"))]
    async fn get(&self, expand: bool) -> Result<(), RpcError>;
}

struct RpcError;

fn main() {}
