use crate::filter::FusenFilter;
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
        let aspect_handler = self
            .handle_context
            .get_controller(&context.context_info.get_handler_key())
            .get_aspect();
        context.insert_server_type(self.server_type.clone());
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
