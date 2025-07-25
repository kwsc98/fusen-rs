use crate::{error::FusenError, handler::HandlerContext};
use std::{net::Shutdown, sync::Arc};

#[derive(Clone)]
pub struct TcpServer;

// impl TcpServer {
//     pub async fn run(
//         port: u16,
//         handler_context: Arc<HandlerContext>,
//         shutdown: Shutdown,
//     ) -> Result<(), FusenError> {

//     }
// }
