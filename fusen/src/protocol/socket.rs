use std::sync::Arc;

use bytes::Bytes;
use fusen_common::{error::FusenError, net::get_path};
use http::{response, Request, Response, Version};
use http_body_util::Full;
use hyper::{body::Incoming, client::conn::http2::SendRequest};
use tokio::{net::TcpStream, sync::RwLock};
use tracing::error;

use crate::{
    register::Resource,
    support::{TokioExecutor, TokioIo},
};

pub struct SocketAssets {
    pub resource: Resource,
    pub socket: Socket,
}

pub enum Socket {
    HTTP1,
    HTTP2(Arc<RwLock<Option<SendRequest<Full<Bytes>>>>>),
}

impl SocketAssets {
    pub async fn send_request(
        &self,
        mut request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, FusenError> {
        match &self.socket {
            Socket::HTTP1 => {
                *request.version_mut() = Version::HTTP_10;
                let io = get_tcp_stream(&self.resource)
                    .await
                    .map_err(|e| FusenError::Client(e.to_string()))?;
                let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
                    .handshake(io)
                    .await
                    .map_err(|e| FusenError::Client(e.to_string()))?;
                tokio::spawn(async move {
                    if let Err(err) = conn.await {
                        error!("conn err : {}", err);
                    }
                });
                let response = sender.send_request(request).await.map_err(|e| {
                    error!("error : {:?}", e);
                    FusenError::Client(e.to_string())
                })?;
                Ok(response)
            }
            Socket::HTTP2(sender_lock) => {
                let sender_read = sender_lock.read().await;
                let mut sender = match sender_read.as_ref() {
                    Some(sender) => sender.clone(),
                    None => {
                        drop(sender_read);
                        let mut sender_write = sender_lock.write().await;
                        let sender = match sender_write.as_ref() {
                            Some(sender) => sender.clone(),
                            None => {
                                let io = get_tcp_stream(&self.resource)
                                    .await
                                    .map_err(|e| FusenError::Client(e.to_string()))?;
                                let (sender, conn) =
                                    hyper::client::conn::http2::Builder::new(TokioExecutor)
                                        .adaptive_window(true)
                                        .handshake(io)
                                        .await
                                        .map_err(|e| FusenError::Client(e.to_string()))?;
                                let sender_lock = sender_lock.clone();
                                tokio::spawn(async move {
                                    let sender = sender_lock;
                                    if let Err(err) = conn.await {
                                        sender.write().await.take();
                                        error!("conn err : {}", err);
                                    }
                                });
                                let _ = sender_write.insert(sender.clone());
                                sender
                            }
                        };
                        sender
                    }
                };
                let response = sender.send_request(request).await.map_err(|e| {
                    error!("{:?}", e);
                    FusenError::Client(e.to_string())
                })?;
                Ok(response)
            }
        }
    }
}

async fn get_tcp_stream(resource: &Resource) -> Result<TokioIo<TcpStream>, crate::Error> {
    let url = get_path(resource.ip.clone(), resource.port.as_deref())
        .parse::<hyper::Uri>()
        .map_err(|e| FusenError::Client(e.to_string()))?;
    let host = url.host().expect("uri has no host");
    let port = url.port_u16().unwrap_or(80);
    let addr = format!("{}:{}", host, port);
    Ok(TcpStream::connect(addr)
        .await
        .map(TokioIo::new)
        .map_err(|e| FusenError::Client(e.to_string()))?)
}
