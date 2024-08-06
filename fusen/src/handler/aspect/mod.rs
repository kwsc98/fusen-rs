use std::sync::Arc;

use crate::codec::request_codec::RequestCodec;
use crate::codec::response_codec::ResponseCodec;
use crate::register::ResourceInfo;
use crate::route::client::Route;
use crate::{
    codec::{request_codec::RequestHandler, response_codec::ResponseHandler},
    filter::FusenFilter,
    FusenFuture,
};
use fusen_common::error::FusenError;
use fusen_common::FusenContext;
use http_body_util::BodyExt;

use super::HandlerContext;

#[allow(async_fn_in_trait)]
pub trait Aspect {
    async fn aroud(
        &self,
        filter: &'static dyn FusenFilter,
        context: FusenContext,
    ) -> Result<FusenContext, crate::Error>;
}

pub trait Aspect_: Send + Sync {
    fn aroud_(
        &'static self,
        filter: &'static dyn FusenFilter,
        context: FusenContext,
    ) -> FusenFuture<Result<FusenContext, crate::Error>>;
}

pub struct DefaultAspect;

impl Aspect_ for DefaultAspect {
    fn aroud_(
        &'static self,
        filter: &'static dyn FusenFilter,
        context: FusenContext,
    ) -> FusenFuture<Result<FusenContext, crate::Error>> {
        Box::pin(async move { filter.call(context).await })
    }
}

pub struct AspectClientFilter {
    request_handle: RequestHandler,
    response_handle: ResponseHandler,
    handle_context: Arc<HandlerContext>,
    route: Route,
}

impl AspectClientFilter {
    pub fn new(
        request_handle: RequestHandler,
        response_handle: ResponseHandler,
        handle_context: Arc<HandlerContext>,
        route: Route,
    ) -> Self {
        AspectClientFilter {
            request_handle,
            response_handle,
            handle_context,
            route,
        }
    }
}

impl FusenFilter for AspectClientFilter {
    fn call(
        &'static self,
        mut context: FusenContext,
    ) -> FusenFuture<Result<FusenContext, crate::Error>> {
        Box::pin(async move {
            let handler_controller = self
                .handle_context
                .get_controller(&context.context_info.get_handler_key());
            let resource_info: ResourceInfo = self
                .route
                .get_server_resource(&context)
                .await
                .map_err(|e| FusenError::Info(e.to_string()))?;
            let ResourceInfo { socket } = resource_info;
            let socket = handler_controller
                .as_ref()
                .get_load_balance()
                .select_(socket)
                .await?;
            let request = self.request_handle.encode(&context)?;
            let response: http::Response<hyper::body::Incoming> =
                socket.send_request(request).await?;
            let res = self
                .response_handle
                .decode(response.map(|e| e.boxed()))
                .await;
            context.response.response = res;
            Ok(context)
        })
    }
}
