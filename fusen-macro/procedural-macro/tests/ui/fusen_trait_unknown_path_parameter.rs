use fusen_procedural_macro::{asset, fusen_trait};

#[fusen_trait]
trait Demo {
    #[asset(path = "/users/{id}", method = GET)]
    async fn get(&self, name: String) -> String;
}

fn main() {}
