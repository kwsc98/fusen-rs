use crate::register::{Info, Register, Resource};
use crate::support::TokioExecutor;
use crate::{register::SocketInfo, support::TokioIo};
use http_body_util::Full;
use hyper::client::conn::http2::SendRequest;
use rand::seq::SliceRandom;
use std::collections::HashSet;
use std::{collections::HashMap, sync::Arc};
use tokio::{net::TcpStream, sync::RwLock};

pub struct Route {
    register: Box<dyn Register>,
    map: Arc<RwLock<HashMap<String, Vec<SocketInfo>>>>,
    client_resource: RwLock<HashSet<String>>,
}

impl Route {
    pub fn new(
        map: Arc<RwLock<HashMap<String, Vec<SocketInfo>>>>,
        register: Box<dyn Register>,
    ) -> Self {
        let client_resource = RwLock::new(HashSet::new());
        Route {
            map,
            register,
            client_resource,
        }
    }

    pub async fn get_socket_sender(
        &self,
        class_name: &str,
        version: Option<&str>,
    ) -> crate::Result<SendRequest<Full<bytes::Bytes>>> {
        let vec_info: Vec<SocketInfo>;
        loop {
            let map = self.map.read().await;
            let mut key = String::from(class_name);
            if let Some(version) = version {
                key.push_str(":");
                key.push_str(version);
            }
            let value = map.get(&key);
            match value {
                Some(value) => {
                    vec_info = value.clone();
                    break;
                }
                None => {
                    drop(map);
                    let resource_client = Resource::Client(Info {
                        server_name: class_name.to_string(),
                        version: version.map(|e|e.to_string()),
                        methods: vec![],
                        ip: fusen_common::get_ip(),
                        port: None,
                    });
                    let read_lock = self.client_resource.read().await;
                    let value = read_lock.get(&key);
                    if let None = value {
                        drop(read_lock);
                        let mut write_lock = self.client_resource.write().await;
                        if let None = write_lock.get(&key) {
                            self.register.add_resource(resource_client);
                            write_lock.insert(key);
                            drop(write_lock);
                        }
                    }
                }
            }
        }
        let socket_info = vec_info
            .choose(&mut rand::thread_rng())
            .ok_or("Not Find Server Node")?;
        let sender = &socket_info.sender;
        let sender_read_lock = sender.read().await;
        if let Some(socket_sender) = sender_read_lock.clone() {
            return Ok(socket_sender);
        } else {
            drop(sender_read_lock);
            let mut sender_write_lock = sender.write().await;
            let sender = match sender_write_lock.as_ref() {
                Some(sender) => sender.clone(),
                None => {
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
                    let (sender_requset, conn) =
                        hyper::client::conn::http2::Builder::new(TokioExecutor)
                            .adaptive_window(true)
                            .handshake(stream)
                            .await?;
                    let sender = sender.clone();
                    tokio::spawn(async move {
                        let sender = sender;
                        if let Err(_err) = conn.await {
                            sender.write().await.take();
                        }
                    });
                    let _ = sender_write_lock.insert(sender_requset.clone());
                    sender_requset
                }
            };
            return Ok(sender);
        }
    }
}
