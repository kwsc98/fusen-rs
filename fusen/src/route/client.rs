use crate::register::{Category, Directory, Register, Resource};
use crate::support::TokioExecutor;
use crate::{register::SocketInfo, support::TokioIo};
use http_body_util::Full;
use hyper::client::conn::http2::SendRequest;
use rand::seq::SliceRandom;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::{net::TcpStream, sync::RwLock};

pub struct Route {
    register: Box<dyn Register>,
    socket_set: RwLock<HashMap<String, SocketInfo>>,
    directory_map: RwLock<HashMap<String, Directory>>,
}

impl Route {
    pub fn new(register: Box<dyn Register>) -> Self {
        let socket_set = RwLock::new(HashMap::new());
        let directory_map = RwLock::new(HashMap::new());
        Route {
            register,
            socket_set,
            directory_map,
        }
    }

    pub async fn get_socket_sender(
        &self,
        class_name: &str,
        version: Option<&str>,
    ) -> crate::Result<SendRequest<Full<bytes::Bytes>>> {
        let vec_info: Arc<Vec<Resource>>;
        let mut key = String::from(class_name);
        if let Some(version) = version {
            key.push_str(":");
            key.push_str(version);
        }
        loop {
            let resource_map_read = self.directory_map.read().await;
            match resource_map_read.get(&key) {
                Some(directory) => {
                    vec_info = directory.get().await?;
                    break;
                }
                None => {
                    drop(resource_map_read);
                    let resource_client = Resource {
                        server_name: class_name.to_string(),
                        category: Category::Client,
                        group: None,
                        version: version.map(|e| e.to_string()),
                        methods: vec![],
                        ip: fusen_common::net::get_ip(),
                        port: None,
                        params: HashMap::new(),
                    };
                    let mut directory_map_write = self.directory_map.write().await;
                    if let None = directory_map_write.get(&key) {
                        let directory = self.register.subscribe(resource_client).await?;
                        directory_map_write.insert(key.clone(), directory);
                    }
                }
            }
        }
        let server_resource = vec_info.iter().fold(vec![], |mut vec, e| {
            if let Category::Server = e.category {
                vec.push(e);
            }
            vec
        });
        let server_resource = server_resource
            .choose(&mut rand::thread_rng())
            .ok_or("Not Find Server Node")?;
        let socket_read = self.socket_set.read().await;
        let ip = server_resource.get_addr();
        let socket_info = match socket_read.get(&ip) {
            Some(info) => info.clone(),
            None => {
                drop(socket_read);
                let socket_info = SocketInfo {
                    sender: Arc::new(RwLock::new(None)),
                };
                let mut socket_map_write = self.socket_set.write().await;
                match socket_map_write.get(&ip) {
                    Some(socket_info) => socket_info.clone(),
                    None => {
                        socket_map_write.insert(ip.to_string(), socket_info.clone());
                        socket_info
                    }
                }
            }
        };
        let sender_read_lock = socket_info.sender.read().await;
        if let Some(socket_sender) = sender_read_lock.clone() {
            return Ok(socket_sender);
        } else {
            drop(sender_read_lock);
            let mut sender_write_lock = socket_info.sender.write().await;
            let sender = match sender_write_lock.as_ref() {
                Some(sender) => sender.clone(),
                None => {
                    let url = ip.to_string().parse::<hyper::Uri>()?;
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
                    let sender = socket_info.sender.clone();
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
