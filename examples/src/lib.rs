//! Shared clean-slate interface contracts and DTOs used by every example binary.

use fusen_rs::{Response, SensitiveFields, interface};
use serde::{Deserialize, Serialize};
/// Reusable runtime extension examples.
pub mod extensions;
/// Interface implementations shared by direct and registry-backed servers.
pub mod service;

/// Demo request payload.
#[derive(Serialize, Deserialize, Default, Debug, SensitiveFields)]
pub struct RequestDto {
    /// Example application value.
    pub str: String,
}

/// Demo response payload.
#[derive(Serialize, Deserialize, Default, Debug, SensitiveFields)]
pub struct ResponseDto {
    /// Example application value.
    pub str: String,
}

/// Primary demonstration interface exposed through the `http-json-v1` binding.
#[interface(name = "demo")]
pub trait DemoService {
    /// Returns a static greeting.
    #[fusen_rs::method(method = "GET", path = "/hello/v4")]
    async fn say_hello_v4(&self) -> Result<Response<String>, fusen_rs::Error>;

    /// Greets one caller by name.
    #[fusen_rs::method(method = "GET", path = "/hello/{name}")]
    async fn say_hello(&self, name: String) -> Result<Response<String>, fusen_rs::Error>;

    /// Demonstrates a JSON request and response body.
    #[fusen_rs::method(method = "POST", path = "/hello/v2")]
    async fn say_hello_v2(
        &self,
        #[param(body)] request: RequestDto,
    ) -> Result<Response<ResponseDto>, fusen_rs::Error>;

    /// Divides two integers or returns a typed invalid-argument error.
    #[fusen_rs::method(method = "GET", path = "/divide")]
    async fn divide(
        &self,
        #[param(query)] a: i32,
        #[param(query)] b: i32,
    ) -> Result<Response<String>, fusen_rs::Error>;
}

/// Versioned secondary interface used to demonstrate discovery identities.
#[interface(name = "demo-v2", group = "v1", version = "1.0")]
pub trait DemoServiceV2 {
    /// Returns a versioned greeting payload.
    #[fusen_rs::method(method = "POST", path = "/hello/v3")]
    async fn say_hello_v3(
        &self,
        #[param(body)] request: RequestDto,
    ) -> Result<Response<ResponseDto>, fusen_rs::Error>;
}
