use fusen_procedural_macro::fusen_trait;

#[fusen_trait]
trait Demo<T> {
    async fn call(&self, value: T) -> String;
}

fn main() {}
