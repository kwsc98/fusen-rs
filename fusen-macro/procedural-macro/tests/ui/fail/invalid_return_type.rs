use fusen_procedural_macro::service;

#[service(name = "user")]
trait UserService {
    async fn get(&self) -> Result<(), String>;
}

fn main() {}
