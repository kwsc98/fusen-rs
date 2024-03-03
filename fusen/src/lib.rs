pub mod client;
mod filter;
pub mod r#macro;
pub mod protocol;
pub mod server;
pub mod support;
pub mod handler;
pub mod register;
pub mod route;

pub type Error = fusen_common::Error;
pub type Result<T> = fusen_common::Result<T>;
pub type FusenFuture<T> = fusen_common::FusenFuture<T>;
