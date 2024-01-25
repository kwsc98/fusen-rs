use crate::support::TokioExecutor;
use crate::{register::SocketInfo, support::TokioIo};
use http_body_util::Full;
use hyper::client::conn::http2::SendRequest;
use rand::seq::SliceRandom;
use std::{collections::HashMap, sync::Arc};
use tokio::io::{self, AsyncWriteExt};
use tokio::{net::TcpStream, sync::RwLock};

pub struct Route {
    map: Arc<RwLock<HashMap<String, Vec<SocketInfo>>>>,
}

impl Route {
    pub fn new(map: Arc<RwLock<HashMap<String, Vec<SocketInfo>>>>) -> Self {
        Route { map }
    }
    
    pub async fn get_socket_sender(
        &self,
        class_name: &str,
        version: &str,
    ) -> crate::Result<SendRequest<Full<bytes::Bytes>>> {
        let map = self.map.read().await;
        let vec_info = map
            .get(&(class_name.to_owned() + ":" + version))
            .ok_or("Not Find Server Node")?
            .clone();
        drop(map);
        let socket_info = vec_info
            .choose(&mut rand::thread_rng())
            .ok_or("Not Find Server Node")?;
        let sender = &socket_info.sender;
        let sender_read_lock = sender.read().await;
        if let Some(socket_sender) = sender_read_lock.to_owned() {
            return Ok(socket_sender);
        } else {
            let url = socket_info
                .info
                .get_addr()
                .to_string()
                .parse::<hyper::Uri>()?;
            let host = url.host().expect("uri has no host");
            let port = url.port_u16().unwrap_or(80);
            let addr = format!("{}:{}", host, port);
            let stream = TcpStream::connect(addr).await?;
            let stream = TokioIo::new(stream);
            let (sender_requset, conn) 
            = hyper::client::conn::http2::Builder::new(TokioExecutor)
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
            drop(sender_read_lock);
            let mut sender_write_lock = sender.write().await;
            let _ = sender_write_lock.insert(sender_requset.clone());
            return Ok(sender_requset);
        }
    }
}
