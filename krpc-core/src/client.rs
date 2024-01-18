use crate::support::{TokioExecutor, TokioIo};
use http::Request;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http2::SendRequest;
use krpc_common::{KrpcMsg, Response};
use serde::{Deserialize, Serialize};
use tokio::io;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::RwLock;

pub struct KrpcClient {
    addr: String,
    socket_sender: RwLock<Option<SendRequest<Full<bytes::Bytes>>>>,
}

impl KrpcClient {
    pub fn build(addr: String) -> KrpcClient {
        let cli = KrpcClient {
            addr,
            socket_sender: RwLock::new(None),
        };
        return cli;
    }

    pub async fn invoke<Req, Res>(&self, msg: KrpcMsg) -> Response<Res>
    where
        Req: Send + Sync + Serialize,
        Res: Send + Sync + Serialize + for<'a> Deserialize<'a> + Default,
    {
        let mut sender 
        = self.get_socket_sender().await.map_err(|e|e.to_string())?;
        let req = Request::builder()
            .header("unique_identifier", msg.unique_identifier)
            .header("version", msg.version)
            .header("class_name", msg.class_name)
            .header("method_name", msg.method_name)
            .body(Full::<bytes::Bytes>::from(msg.req)).map_err(|e|e.to_string())?;
        let mut res = sender.send_request(req).await.map_err(|e|e.to_string())?;
        let res: Response<String> = serde_json::from_slice(
            res.frame()
                .await
                .unwrap()
                .map_err(|e|e.to_string())?
                .data_ref()
                .unwrap()
                .as_ref(),
        )
        .map_err(|e|e.to_string())?;
        let res: Result<Res, String> = match res {
            Ok(data) => Ok(serde_json::from_slice(&data.as_bytes()).map_err(|e|e.to_string())?),
            Err(info) => Err(info),
        };
        return res;
    }

    async fn get_socket_sender(&self) -> Result<SendRequest<Full<bytes::Bytes>>,crate::Error> {
        let socket_sender = self.socket_sender.read().await;
        if socket_sender.is_none() {
            drop(socket_sender);
            let mut socket_sender = self.socket_sender.write().await;
            if socket_sender.is_none() {
                let url = self.addr.parse::<hyper::Uri>()?;
                let host = url.host().expect("uri has no host");
                let port = url.port_u16().unwrap_or(80);
                let addr = format!("{}:{}", host, port);
                let stream = TcpStream::connect(addr).await?;
                let stream = TokioIo::new(stream);
                let (sender, conn) = hyper::client::conn::http2::Builder::new(TokioExecutor)
                    .adaptive_window(true)
                    .handshake(stream)
                    .await?;
                tokio::spawn(async move {
                    if let Err(err) = conn.await {
                        let mut stdout = io::stdout();
                        stdout
                            .write_all(format!("Connection failed: {:?}", err).as_bytes())
                            .await
                            .unwrap();
                        stdout.flush().await.unwrap();
                    }
                });
                let _ = socket_sender.insert(sender);
                return Ok(socket_sender.as_mut().unwrap().clone());
            }
            return Ok(socket_sender.as_ref().unwrap().clone());
        }
        return Ok(socket_sender.as_ref().unwrap().clone());
    }
}
