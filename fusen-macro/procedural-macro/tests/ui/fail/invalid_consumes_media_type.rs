use fusen_procedural_macro::interface;

#[interface(name = "invalid-consumes")]
trait InvalidConsumes {
    #[fusen_procedural_macro::method(
        method = "POST",
        path = "/items",
        consumes = "not a media type"
    )]
    async fn create(&self) -> Result<Response<String>, Error>;
}

fn main() {}
