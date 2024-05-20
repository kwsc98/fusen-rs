use crate::codec::request_codec::RequestCodec;
use crate::codec::request_codec::RequestHandler;
use crate::codec::response_codec::ResponseCodec;
use crate::codec::response_codec::ResponseHandler;
use crate::handler::HandlerContext;
use crate::register::Register;
use crate::register::ResourceInfo;
use crate::route::client::Route;
use fusen_common::codec::json_field_compatible;
use fusen_common::error::FusenError;
use fusen_common::FusenContext;
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub struct FusenClient {
    request_handle: RequestHandler,
    response_handle: ResponseHandler,
    handle_context: HandlerContext,
    route: Route,
}

impl FusenClient {
    pub fn build(register: Arc<Box<dyn Register>>, handle_context: HandlerContext) -> FusenClient {
        FusenClient {
            request_handle: RequestHandler::new(),
            response_handle: ResponseHandler::new(),
            handle_context,
            route: Route::new(register),
        }
    }

    pub async fn invoke<Res>(&self, mut context: FusenContext) -> Result<Res, FusenError>
    where
        Res: Send + Sync + Serialize + for<'a> Deserialize<'a> + Default,
    {
        let handler_controller = self
            .handle_context
            .get_controller(&context.context_info.get_handler_key())
            .ok_or_else(|| FusenError::from("not find handler_controller"))?;
        let resource_info: ResourceInfo = self
            .route
            .get_server_resource(&context)
            .await
            .map_err(|e| FusenError::Info(e.to_string()))?;
        let ResourceInfo {
            server_type,
            socket,
        } = resource_info;
        context.insert_server_type(server_type);
        let return_ty = context.get_return_ty().unwrap();
        let socket = handler_controller
            .as_ref()
            .get_load_balance()
            .select_(socket)
            .await?;
        let request = self.request_handle.encode(context)?;
        let response: http::Response<hyper::body::Incoming> = socket.send_request(request).await?;
        let res = self
            .response_handle
            .decode(response.map(|e| e.boxed()))
            .await?;
        let response = json_field_compatible(return_ty, res)?;
        let response: Res =
            serde_json::from_str(&response).map_err(|e| FusenError::from(e.to_string()))?;
        Ok(response)
    }
}
