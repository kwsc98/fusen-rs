pub mod error;
pub mod resource;
pub mod utils;

pub type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
