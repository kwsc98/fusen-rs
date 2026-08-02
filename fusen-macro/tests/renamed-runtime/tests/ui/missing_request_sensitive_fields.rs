use runtime::interface;

#[derive(serde::Serialize, serde::Deserialize)]
struct Request {
    value: String,
}

#[interface(name = "missing-request-sensitivity")]
trait MissingRequestSensitivity {
    #[runtime::method(method = "POST", path = "/request")]
    async fn call(
        &self,
        #[param(body)] request: Request,
    ) -> Result<runtime::Response<String>, runtime::Error>;
}

fn main() {}
