use fusen_procedural_macro::fusen_trait;

#[fusen_trait]
trait Demo {
    async fn call(&mut self) -> String;
}

fn main() {}
