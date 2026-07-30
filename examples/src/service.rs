//! Direct implementations of the generated interface traits.

use crate::{
    DemoService, DemoServiceV2, DivideRequest, GreetingRequest, HelloRequest, ResponseDto,
};
use fusen_rs::{RpcCategory, RpcError, RpcRequest, RpcResponse};

/// Primary demonstration interface implementation.
#[derive(Debug, Default)]
pub struct DemoServiceImpl;

impl DemoService for DemoServiceImpl {
    async fn say_hello_v4(
        &self,
        _request: RpcRequest<()>,
    ) -> Result<RpcResponse<String>, RpcError> {
        Ok(RpcResponse::new("Hello V4".to_string()))
    }

    async fn say_hello(
        &self,
        request: RpcRequest<HelloRequest>,
    ) -> Result<RpcResponse<String>, RpcError> {
        Ok(RpcResponse::new(format!(
            "Hello {}",
            request.into_body().name
        )))
    }

    async fn say_hello_v2(
        &self,
        request: RpcRequest<GreetingRequest>,
    ) -> Result<RpcResponse<ResponseDto>, RpcError> {
        Ok(RpcResponse::new(ResponseDto {
            str: format!("HelloV2 {}", request.into_body().request.str),
        }))
    }

    async fn divide(
        &self,
        request: RpcRequest<DivideRequest>,
    ) -> Result<RpcResponse<String>, RpcError> {
        let DivideRequest { a, b } = request.into_body();
        if b == 0 {
            return Err(RpcError::new(
                RpcCategory::InvalidArgument,
                "zero_divisor",
                "divisor must not be zero",
            )
            .expect("the static error code is valid"));
        }
        Ok(RpcResponse::new(format!("a / b = {}", a / b)))
    }
}

/// Secondary versioned interface implementation.
#[derive(Debug, Default)]
pub struct DemoServiceImplV2;

impl DemoServiceV2 for DemoServiceImplV2 {
    async fn say_hello_v3(
        &self,
        request: RpcRequest<GreetingRequest>,
    ) -> Result<RpcResponse<ResponseDto>, RpcError> {
        Ok(RpcResponse::new(ResponseDto {
            str: format!("HelloV3 {}", request.into_body().request.str),
        }))
    }
}
