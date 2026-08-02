use fusen_procedural_macro::interface;

struct Error;
struct Response<T>(T);

#[interface(name = "removed-idempotency")]
pub trait RemovedIdempotencyApi {
    #[fusen_procedural_macro::method(idempotency = "safe")]
    async fn get(&self) -> Result<Response<String>, Error>;
}

fn main() {}
