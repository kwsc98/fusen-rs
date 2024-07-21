use crate::handler::HandlerInfo;

pub struct FusenApplicationConfig {
    application_name: String,
    port: String,
    register: Vec<String>,
    handler_infos: Vec<HandlerInfo>,
}
