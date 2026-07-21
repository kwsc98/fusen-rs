use rpc::{
    error::FusenError,
    fusen_procedural_macro::{fusen_service, fusen_trait},
};

#[fusen_trait]
trait First {
    async fn first(&self) -> String;
}

#[fusen_trait]
trait Second {
    async fn second(&self) -> String;
}

struct Service;

#[fusen_service]
impl First for Service {
    async fn first(&self) -> Result<String, FusenError> {
        Ok("first".into())
    }
}

#[fusen_service]
impl Second for Service {
    async fn second(&self) -> Result<String, FusenError> {
        Ok("second".into())
    }
}

fn main() {}
