pub mod client;
mod filter;
pub mod protocol;
pub mod server;
pub mod support;
pub mod common;
pub mod handler;
pub mod register;
pub mod r#macro;

pub type Error = krpc_common::Error;
pub type Result<T> = krpc_common::Result<T>;
pub type KrpcFuture<T> = krpc_common::KrpcFuture<T>;
