use crate::{DemoService, DemoServiceV2, RequestDto, ResponseDto};
use fusen_rs::{error::FusenError, fusen_procedural_macro::fusen_service};

#[derive(Debug, Default)]
pub struct DemoServiceImpl;

#[fusen_service]
impl DemoService for DemoServiceImpl {
    async fn sayHelloV4(&self) -> Result<String, FusenError> {
        Ok("Hello V4".to_string())
    }

    async fn sayHello(&self, name: String) -> Result<String, FusenError> {
        Ok(format!("Hello {name}"))
    }

    async fn sayHelloV2(&self, name: RequestDto) -> Result<ResponseDto, FusenError> {
        Ok(ResponseDto {
            str: format!("HelloV2 {}", name.str),
        })
    }

    async fn divideV2(&self, a: i32, b: i32) -> Result<String, FusenError> {
        if b == 0 {
            return Err(FusenError::InvalidRequest(
                "divisor must not be zero".to_owned(),
            ));
        }
        Ok(format!("a / b = {}", a / b))
    }
}

#[derive(Debug, Default)]
pub struct DemoServiceImplV2;

#[fusen_service]
impl DemoServiceV2 for DemoServiceImplV2 {
    async fn sayHelloV3(&self, name: RequestDto) -> Result<ResponseDto, FusenError> {
        Ok(ResponseDto {
            str: format!("HelloV3 {}", name.str),
        })
    }
}
