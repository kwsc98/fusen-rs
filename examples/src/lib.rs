//! Shared clean-slate interface contracts and DTOs used by every example binary.

use fusen_rs::interface;
use serde::{Deserialize, Serialize};
/// Reusable middleware and extension examples.
pub mod middleware;
/// Interface implementations shared by direct and registry-backed servers.
pub mod service;

/// Demo request payload.
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct RequestDto {
    /// Example application value.
    pub str: String,
}

/// Demo response payload.
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ResponseDto {
    /// Example application value.
    pub str: String,
}

/// Primary demonstration interface available through both versioned wire protocols.
#[interface(name = "demo")]
pub trait DemoService {
    /// Returns a static greeting.
    #[fusen_rs::method(idempotency = "safe", spring(method = "GET", path = "/hello/v4"))]
    async fn say_hello_v4(&self) -> Result<RpcResponse<String>, fusen_rs::RpcError>;

    /// Greets one caller by name.
    #[fusen_rs::method(idempotency = "safe", spring(method = "GET", path = "/hello/{name}"))]
    async fn say_hello(
        &self,
        #[rpc(path)] name: String,
    ) -> Result<RpcResponse<String>, fusen_rs::RpcError>;

    /// Demonstrates a JSON request and response body.
    #[fusen_rs::method(idempotency = "none", spring(method = "POST", path = "/hello/v2"))]
    async fn say_hello_v2(
        &self,
        #[rpc(body)] request: RequestDto,
    ) -> Result<RpcResponse<ResponseDto>, fusen_rs::RpcError>;

    /// Divides two integers or returns a typed invalid-argument error.
    #[fusen_rs::method(idempotency = "safe", spring(method = "GET", path = "/divide"))]
    async fn divide(
        &self,
        #[rpc(query)] a: i32,
        #[rpc(query)] b: i32,
    ) -> Result<RpcResponse<String>, fusen_rs::RpcError>;
}

/// Versioned secondary interface used to demonstrate discovery identities.
#[interface(name = "demo-v2", group = "v1", version = "1.0")]
pub trait DemoServiceV2 {
    /// Returns a versioned greeting payload.
    #[fusen_rs::method(idempotency = "none", spring(method = "POST", path = "/hello/v3"))]
    async fn say_hello_v3(
        &self,
        #[rpc(body)] request: RequestDto,
    ) -> Result<RpcResponse<ResponseDto>, fusen_rs::RpcError>;
}
