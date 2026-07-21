use fusen_procedural_macro::fusen_trait;

#[fusen_trait]
trait Demo {
    type Output;

    async fn call(&self) -> String;
}

fn main() {}
