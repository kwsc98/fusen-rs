//! Shared clean-slate service contracts and DTOs used by every example binary.

use fusen_rs::service;
use serde::{Deserialize, Serialize};
/// Reusable middleware and extension examples.
pub mod middleware;
/// Service implementations shared by direct and registry-backed servers.
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

/// Primary demonstration service available through both versioned wire protocols.
#[service(name = "demo")]
pub trait DemoService {
    /// Returns a static greeting.
    #[fusen_rs::method(idempotency = "safe", spring(method = "GET", path = "/hello/v4"))]
    async fn say_hello_v4(&self) -> Result<String, fusen_rs::RpcError>;

    /// Greets one caller by name.
    #[fusen_rs::method(idempotency = "safe", spring(method = "GET", path = "/hello/{name}"))]
    async fn say_hello(&self, name: String) -> Result<String, fusen_rs::RpcError>;

    /// Demonstrates a JSON request and response body.
    #[fusen_rs::method(
        idempotency = "none",
        spring(method = "POST", path = "/hello/v2", body = "request")
    )]
    async fn say_hello_v2(&self, request: RequestDto) -> Result<ResponseDto, fusen_rs::RpcError>;

    /// Divides two integers or returns a typed invalid-argument error.
    #[fusen_rs::method(
        idempotency = "safe",
        spring(method = "GET", path = "/divide", query = ["a", "b"])
    )]
    async fn divide(&self, a: i32, b: i32) -> Result<String, fusen_rs::RpcError>;
}

/// Versioned secondary service used to demonstrate discovery identities.
#[service(name = "demo-v2", group = "v1", version = "1.0")]
pub trait DemoServiceV2 {
    /// Returns a versioned greeting payload.
    #[fusen_rs::method(
        idempotency = "none",
        spring(method = "POST", path = "/hello/v3", body = "request")
    )]
    async fn say_hello_v3(&self, request: RequestDto) -> Result<ResponseDto, fusen_rs::RpcError>;
}
