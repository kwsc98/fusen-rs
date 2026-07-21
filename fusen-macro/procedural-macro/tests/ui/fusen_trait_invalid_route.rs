use fusen_procedural_macro::fusen_trait;

#[fusen_trait]
trait Demo {
    #[fusen_procedural_macro::asset(path = "/items?all=true")]
    async fn call(&self) -> String;
}

fn main() {}
