//! Direct implementations of the generated interface traits.

use crate::{DemoService, DemoServiceV2, RequestDto, ResponseDto};
use fusen_rs::{Error, ErrorCategory, Response};

/// Primary demonstration interface implementation.
#[derive(Debug, Default)]
pub struct DemoServiceImpl;

impl DemoService for DemoServiceImpl {
    async fn say_hello_v4(&self) -> Result<Response<String>, Error> {
        Ok(Response::new("Hello V4".to_string()))
    }

    async fn say_hello(&self, name: String) -> Result<Response<String>, Error> {
        Ok(Response::new(format!("Hello {}", name)))
    }

    async fn say_hello_v2(&self, request: RequestDto) -> Result<Response<ResponseDto>, Error> {
        Ok(Response::new(ResponseDto {
            str: format!("HelloV2 {}", request.str),
        }))
    }

    async fn divide(&self, a: i32, b: i32) -> Result<Response<String>, Error> {
        if b == 0 {
            return Err(Error::application(
                ErrorCategory::InvalidArgument,
                "zero_divisor",
                "divisor must not be zero",
            )
            .expect("the static error code is valid"));
        }
        Ok(Response::new(format!("a / b = {}", a / b)))
    }
}

/// Secondary versioned interface implementation.
#[derive(Debug, Default)]
pub struct DemoServiceImplV2;

impl DemoServiceV2 for DemoServiceImplV2 {
    async fn say_hello_v3(&self, request: RequestDto) -> Result<Response<ResponseDto>, Error> {
        Ok(Response::new(ResponseDto {
            str: format!("HelloV3 {}", request.str),
        }))
    }
}
