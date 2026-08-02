use fusen_procedural_macro::interface;

#[interface(name = "invalid-produces")]
trait InvalidProduces {
    #[fusen_procedural_macro::method(
        method = "GET",
        path = "/items",
        produces = "application/json, text/plain"
    )]
    async fn list(&self) -> Result<Response<String>, Error>;
}

fn main() {}
