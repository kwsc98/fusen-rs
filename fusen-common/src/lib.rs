pub mod resource;
pub mod utils;
pub mod protocol;
pub type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;

pub use serde_json;
