//! Direct implementations of the generated service traits.

use crate::{DemoService, DemoServiceV2, RequestDto, ResponseDto};
use fusen_rs::{RpcCategory, RpcError};

/// Primary demonstration service implementation.
#[derive(Debug, Default)]
pub struct DemoServiceImpl;

impl DemoService for DemoServiceImpl {
    async fn say_hello_v4(&self) -> Result<String, RpcError> {
        Ok("Hello V4".to_string())
    }

    async fn say_hello(&self, name: String) -> Result<String, RpcError> {
        Ok(format!("Hello {name}"))
    }

    async fn say_hello_v2(&self, request: RequestDto) -> Result<ResponseDto, RpcError> {
        Ok(ResponseDto {
            str: format!("HelloV2 {}", request.str),
        })
    }

    async fn divide(&self, a: i32, b: i32) -> Result<String, RpcError> {
        if b == 0 {
            return Err(RpcError::new(
                RpcCategory::InvalidArgument,
                "zero_divisor",
                "divisor must not be zero",
            )
            .expect("the static error code is valid"));
        }
        Ok(format!("a / b = {}", a / b))
    }
}

/// Secondary versioned service implementation.
#[derive(Debug, Default)]
pub struct DemoServiceImplV2;

impl DemoServiceV2 for DemoServiceImplV2 {
    async fn say_hello_v3(&self, request: RequestDto) -> Result<ResponseDto, RpcError> {
        Ok(ResponseDto {
            str: format!("HelloV3 {}", request.str),
        })
    }
}
