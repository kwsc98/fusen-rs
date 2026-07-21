use fusen_procedural_macro::fusen_trait;

#[fusen_trait]
trait Demo {
    async fn call<'a>(&self, value: String) -> String;
}

fn main() {}
