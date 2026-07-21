use fusen_procedural_macro::fusen_trait;

#[fusen_trait]
#[fusen_procedural_macro::asset(path = "/one")]
#[fusen_procedural_macro::asset(path = "/two")]
trait Demo {
    async fn call(&self) -> String;
}

fn main() {}
