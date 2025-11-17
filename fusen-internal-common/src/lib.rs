pub mod protocol;
pub mod resource;
pub mod utils;

pub type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
pub type BoxFutureV2<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

pub use serde_json;
