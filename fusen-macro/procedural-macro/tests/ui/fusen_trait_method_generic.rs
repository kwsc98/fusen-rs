use fusen_procedural_macro::fusen_trait;

#[fusen_trait]
trait Demo {
    async fn call<T>(&self, value: T) -> String;
}

fn main() {}
