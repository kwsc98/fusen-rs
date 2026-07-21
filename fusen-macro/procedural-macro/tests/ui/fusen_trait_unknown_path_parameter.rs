use fusen_procedural_macro::fusen_trait;

#[fusen_trait]
trait Demo {
    #[fusen_procedural_macro::asset(path = "/users/{id}", method = GET)]
    async fn get(&self, name: String) -> String;
}

fn main() {}
