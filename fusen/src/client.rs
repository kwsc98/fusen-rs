use crate::codec::response_codec::ResponseHandler;
use crate::handler::aspect::AspectClientFilter;
use crate::handler::HandlerContext;
use crate::register::Register;
use crate::route::client::Route;
use crate::{codec::request_codec::RequestHandler, filter::FusenFilter};
use fusen_common::codec::json_field_compatible;
use fusen_common::error::FusenError;
use fusen_common::FusenContext;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub struct FusenClient {
    client_filter: &'static dyn FusenFilter,
    handle_context: Arc<HandlerContext>,
}

impl FusenClient {
    pub fn build(
        register: Arc<Box<dyn Register>>,
        handle_context: Arc<HandlerContext>,
    ) -> FusenClient {
        FusenClient {
            client_filter: Box::leak(Box::new(AspectClientFilter::new(
                RequestHandler::new(HashMap::new()),
                ResponseHandler::new(),
                handle_context.clone(),
                Route::new(register),
            ))),
            handle_context,
        }
    }

    pub async fn invoke<Res>(&self, context: FusenContext) -> Result<Res, FusenError>
    where
        Res: Send + Sync + Serialize + for<'a> Deserialize<'a> + Default,
    {
        let aspect_handler = self
            .handle_context
            .get_controller(&context.context_info.get_handler_key())
            .get_aspect();
        let context = aspect_handler.aroud_(self.client_filter, context).await?;
        let return_ty = context.response.response_ty.unwrap();
        match context.response.response {
            Ok(res) => {
                let response = json_field_compatible(return_ty, res)?;
                let response: Res =
                    serde_json::from_str(&response).map_err(|e| FusenError::from(e.to_string()))?;
                Ok(response)
            }
            Err(err) => Err(err),
        }
    }
}
