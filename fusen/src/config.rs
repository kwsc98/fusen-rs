use fusen_macro::Data;
use serde::{Deserialize, Serialize};

use crate::handler::HandlerInfo;

#[derive(Serialize, Deserialize, Default, Data)]
pub struct FusenApplicationConfig {
    application_name: String,
    port: u16,
    register: String,
    handler_infos: Option<Vec<HandlerInfo>>,
}
