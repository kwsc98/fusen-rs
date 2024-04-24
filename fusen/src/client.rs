use crate::codec::grpc_codec::GrpcBodyCodec;
use crate::codec::json_codec::JsonBodyCodec;
use crate::codec::request_codec::RequestCodec;
use crate::codec::request_codec::RequestHandler;
use crate::codec::response_codec::ResponseCodec;
use crate::codec::response_codec::ResponseHandler;
use crate::register::{RegisterBuilder, ResourceInfo};
use crate::route::client::Route;
use crate::support::triple::{TripleRequestWrapper, TripleResponseWrapper};
use fusen_common::codec::json_field_compatible;
use fusen_common::error::FusenError;
use fusen_common::url::UrlConfig;
use fusen_common::FusenContext;
use http_body_util::BodyExt;
use rand::prelude::SliceRandom;
use serde::{Deserialize, Serialize};

pub struct FusenClient {
    request_handle: RequestHandler,
    response_handle: ResponseHandler,
    route: Route,
}

impl FusenClient {
    pub fn build(register_config: Box<dyn UrlConfig>) -> FusenClient {
        let registry_builder = RegisterBuilder::new(register_config).unwrap();
        let register = registry_builder.init();
        let json_codec = JsonBodyCodec::<bytes::Bytes>::new();
        let grpc_codec =
            GrpcBodyCodec::<bytes::Bytes, TripleResponseWrapper, TripleRequestWrapper>::new();
        FusenClient {
            request_handle: RequestHandler::new(),
            response_handle: ResponseHandler::new(),
            route: Route::new(register),
        }
    }

    pub async fn invoke<Res>(&self, msg: FusenContext, return_ty: &str) -> Result<Res, FusenError>
    where
        Res: Send + Sync + Serialize + for<'a> Deserialize<'a> + Default,
    {
        let resource_info: ResourceInfo = self
            .route
            .get_server_resource(&msg)
            .await
            .map_err(|e| FusenError::Info(e.to_string()))?;
        let ResourceInfo {
            server_type,
            socket,
        } = resource_info;
        let socket = socket
            .choose(&mut rand::thread_rng())
            .ok_or(FusenError::from("not find server"))?;
        let request = self.request_handle.encode(msg)?;
        let response: http::Response<hyper::body::Incoming> = socket.send_request(request).await?;
        let res = self.response_handle.decode(response).await??;
        let res = json_field_compatible(return_ty, res);
        let res: Res = serde_json::from_str(&res).map_err(|e| FusenError::from(e.to_string()))?;
        return Ok(res);
    }
}
