pub mod client;
pub mod codec;
mod filter;
pub mod protocol;
pub mod register;
pub mod route;
pub mod server;
pub mod support;
pub use fusen_common;
pub use fusen_macro;
use std::convert::Infallible;
pub type Error = fusen_common::Error;
pub type Result<T> = fusen_common::Result<T>;
pub type FusenFuture<T> = fusen_common::FusenFuture<T>;
pub type HttpBody = futures_util::stream::Iter<
    std::vec::IntoIter<std::result::Result<http_body::Frame<bytes::Bytes>, Infallible>>,
>;
pub type BoxBody<D, E> = http_body_util::combinators::BoxBody<D, E>;

pub type StreamBody<D, E> = http_body_util::StreamBody<
    futures_util::stream::Iter<std::vec::IntoIter<std::result::Result<http_body::Frame<D>, E>>>,
>;
