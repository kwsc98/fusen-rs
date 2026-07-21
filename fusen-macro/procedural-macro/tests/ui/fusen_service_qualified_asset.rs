use fusen_procedural_macro::fusen_service;

trait Demo {
    async fn call(&self) -> String;
}

struct Service;

#[fusen_service]
#[fusen_procedural_macro::asset(path = "/duplicate")]
impl Demo for Service {
    async fn call(&self) -> String {
        "value".into()
    }
}

fn main() {}
