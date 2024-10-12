use crate::filter::{FusenFilter, ProceedingJoinPoint};
use crate::handler::HandlerContext;
use fusen_common::codec::json_field_compatible;
use fusen_common::error::FusenError;
use fusen_common::register::Type;
use fusen_common::FusenContext;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub struct FusenClient {
    server_type: Type,
    client_filter: &'static dyn FusenFilter,
    handle_context: Arc<HandlerContext>,
}

impl FusenClient {
    pub fn build(
        server_type: Type,
        client_filter: &'static dyn FusenFilter,
        handle_context: Arc<HandlerContext>,
    ) -> FusenClient {
        FusenClient {
            server_type,
            client_filter,
            handle_context,
        }
    }

    pub async fn invoke<Res>(&self, mut context: FusenContext) -> Result<Res, FusenError>
    where
        Res: Send + Sync + Serialize + for<'a> Deserialize<'a> + Default,
    {
        let mut aspect_handler = self
            .handle_context
            .get_controller(&context.get_context_info().get_handler_key())
            .get_aspect();
        context.insert_server_type(self.server_type.clone());
        aspect_handler.push_back(self.client_filter);
        let join_point = ProceedingJoinPoint::new(aspect_handler, context);
        let context = join_point.proceed().await?;
        let return_ty = context.get_response().get_response_ty().unwrap();
        match context.into_response().into_response() {
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
