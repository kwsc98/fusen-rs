use fusen_procedural_macro::interface;

struct RpcError;

#[interface(name = "invalid-result-lifetime")]
trait InvalidResultLifetime {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(&self) -> Result<'static, RpcError>;
}

#[interface(name = "invalid-result-const")]
trait InvalidResultConst {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(&self) -> Result<{ 1 }, RpcError>;
}

#[interface(name = "invalid-result-assoc-type")]
trait InvalidResultAssocType {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(&self) -> Result<Item = String, RpcError>;
}

#[interface(name = "invalid-result-assoc-const")]
trait InvalidResultAssocConst {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(&self) -> Result<COUNT = 1, RpcError>;
}

#[interface(name = "invalid-result-constraint")]
trait InvalidResultConstraint {
    #[fusen_procedural_macro::method(method = "GET", path = "/call")]
    async fn call(&self) -> Result<Item: Send, RpcError>;
}

fn main() {}
