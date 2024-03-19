use futures::Future;
pub mod server;


pub trait FusenFilter {
    type Request;

    type Response: Send;

    type Error: Send;

    type Future: Future<Output = Result<Self::Response, Self::Error>> + Send;

    fn call(&self, req: Self::Request) -> Self::Future;
}
