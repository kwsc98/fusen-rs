pub mod common;

pub mod protocol;

pub mod handler;

mod server;

mod status;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub type Result<T> = std::result::Result<T, Error>;

pub type BoxBody = http_body::combinators::UnsyncBoxBody<bytes::Bytes, status::Status>;