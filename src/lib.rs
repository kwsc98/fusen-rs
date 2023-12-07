pub mod client;
mod filter;
pub mod protocol;
pub mod server;
pub mod support;
pub mod common;
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
pub type KrpcFuture< T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send >>;
