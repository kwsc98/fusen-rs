use fusen_procedural_macro::service;

#[service(name = "user")]
trait UserService {
    #[fusen_procedural_macro::method(spring(
        method = "POST",
        path = "/users/{id}",
        body = "id"
    ))]
    async fn update(&self, id: String) -> Result<(), RpcError>;
}

struct RpcError;

fn main() {}
