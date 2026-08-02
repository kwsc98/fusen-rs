use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "removed-spring-mapping")]
pub trait RemovedSpringMappingApi {
    #[fusen_procedural_macro::method(spring(method = "GET", path = "/users"))]
    async fn get(&self) -> Result<Response<String>, Error>;
}

fn main() {}
