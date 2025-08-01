pub mod resource;
pub mod utils;
pub type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;

pub use serde_json;

#[test]
fn ds() {
    let de = vec!["ds","dsd"];
    let d2 = ["ds","dsd"];
}