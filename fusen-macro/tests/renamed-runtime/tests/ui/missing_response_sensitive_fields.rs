use runtime::interface;

#[derive(serde::Serialize, serde::Deserialize)]
struct Response {
    value: String,
}

#[interface(name = "missing-response-sensitivity")]
trait MissingResponseSensitivity {
    #[runtime::method(method = "GET", path = "/response")]
    async fn call(&self) -> Result<runtime::Response<Response>, runtime::Error>;
}

fn main() {}
