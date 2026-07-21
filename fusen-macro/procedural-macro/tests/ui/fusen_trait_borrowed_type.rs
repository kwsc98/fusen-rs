use fusen_procedural_macro::fusen_trait;

#[fusen_trait]
trait Demo {
    async fn call(&self, value: Option<&str>) -> String;
}

fn main() {}
