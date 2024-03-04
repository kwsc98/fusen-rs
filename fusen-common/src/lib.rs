use std::{
    fmt::{self, Display, Formatter},
    net::IpAddr,
};

use serde::{Deserialize, Serialize};
use tracing_subscriber::fmt::writer::MakeWriterExt;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
pub type Response<T> = std::result::Result<T, String>;
pub type FusenFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
pub type RpcResult<T> = std::result::Result<T, RpcError>;

pub mod date_util;
pub mod url_util;

#[derive(Serialize, Deserialize, Debug)]
pub enum RpcError {
    Null,
    Client(String),
    Server(String),
    Method(String),
}

unsafe impl Send for RpcError {}

unsafe impl Sync for RpcError {}

impl Display for RpcError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::Null => write!(f, "Bad value"),
            RpcError::Client(msg) => write!(f, "RpcError::Client {}", msg),
            RpcError::Server(msg) => write!(f, "RpcError::Server {}", msg),
            RpcError::Method(msg) => write!(f, "RpcError::Method {}", msg),
        }
    }
}

impl std::error::Error for RpcError {}

#[derive(Debug)]
pub struct FusenMsg {
    pub unique_identifier: String,
    pub version: Option<String>,
    pub class_name: String,
    pub method_name: String,
    pub req: Vec<String>,
    pub res: core::result::Result<String, RpcError>,
}

impl FusenMsg {
    pub fn new_empty() -> FusenMsg {
        return FusenMsg {
            unique_identifier: "".to_string(),
            version: None,
            class_name: "".to_string(),
            method_name: "".to_string(),
            req: vec![],
            res: Err(RpcError::Null),
        };
    }

    pub fn new(
        unique_identifier: String,
        version: Option<String>,
        class_name: String,
        method_name: String,
        req: Vec<String>,
        res: core::result::Result<String, RpcError>,
    ) -> FusenMsg {
        return FusenMsg {
            unique_identifier,
            version,
            class_name,
            method_name,
            req,
            res,
        };
    }
}

#[derive(Clone,Debug, Serialize, Deserialize)]
pub struct MethodResource {
    id: String,
    path: String,
    //get ot post
    method: String,
}

impl MethodResource {
    pub fn new(id: String, path: String, method: String) -> Self {
        Self { id, path, method }
    }
    pub fn form_json_str(str: &str) -> Self {
        serde_json::from_str(str).unwrap()
    }
    pub fn to_json_str(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

pub trait RpcServer: Send + Sync {
    fn invoke(&self, msg: FusenMsg) -> FusenFuture<FusenMsg>;
    fn get_info(&self) -> (&str, &str, Option<&str>, Vec<String>);
}

pub fn init_log() {
    let stdout = std::io::stdout.with_max_level(tracing::Level::DEBUG);
    tracing_subscriber::fmt()
        .with_writer(stdout)
        .with_line_number(true)
        .with_thread_ids(true)
        .init();
}

pub fn get_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn get_network_ip() -> std::result::Result<IpAddr, Box<dyn std::error::Error>> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    let local_ip = socket.local_addr()?.ip();
    Ok(local_ip)
}

pub fn get_ip() -> String {
    match get_network_ip() {
        Ok(ok) => ok.to_string(),
        Err(_err) => "127.0.0.1".to_string(),
    }
}
