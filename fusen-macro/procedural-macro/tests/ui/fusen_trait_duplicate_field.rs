use fusen_procedural_macro::fusen_trait;

#[fusen_trait(id = "one", id = "two")]
trait Demo {
    async fn call(&self) -> String;
}

fn main() {}
