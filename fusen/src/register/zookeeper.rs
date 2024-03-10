use super::{Register, Resource, SocketInfo};
use crate::support::dubbo::{decode_url, encode_url};
use async_recursion::async_recursion;
use fusen_common::server::Protocol;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tracing::info;
use zk::OneshotWatcher;
use zookeeper_client as zk;

static EPHEMERAL_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Ephemeral.with_acls(zk::Acls::anyone_all());
static CONTAINER_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Container.with_acls(zk::Acls::anyone_all());

pub struct FusenZookeeper {
    addr: String,

    root_path: String,

    map: Arc<RwLock<HashMap<String, Vec<Resource>>>>,
}

impl Register for FusenZookeeper {
    fn add_resource(&self, resource: Resource) {
        creat_resource_node(
            self.addr.clone(),
            self.root_path.clone(),
            resource,
            self.map.clone(),
        )
    }

    fn check(&self, protocol: &Vec<Protocol>) -> bool {
        for protocol in protocol {
            if let Protocol::HTTP2(_) = protocol {
                return true;
            }
        }
        return false;
    }

    fn get_resource<'a>(
        &'a self,
        key: &str,
    ) -> fusen_common::FusenFuture<Option<&'a Vec<Resource>>> {
        Box::pin(async move {
            let map = self.map.read().await;
            let res = map.get(key);
            res
        })
    }
}

impl FusenZookeeper {
    pub fn init(
        addr: &str,
        _name_space: &str,
        map: Arc<RwLock<HashMap<String, Vec<Resource>>>>,
    ) -> Self {
        let root_path = "/dubbo".to_string();
        let fusen_zookeeper = FusenZookeeper {
            addr: addr.to_string(),
            root_path,
            map,
        };
        return fusen_zookeeper;
    }
}

#[async_recursion]
async fn connect(cluster: &str, chroot: &str) -> zk::Client {
    let client = match zk::Client::connect(&cluster).await {
        Ok(client) => client,
        Err(err) => {
            tokio::time::sleep(Duration::from_secs(30)).await;
            info!("connect err {:?} ,Try to re-establish the connection", err);
            return connect(cluster, chroot).await;
        }
    };
    if chroot.len() <= 1 {
        return client;
    }
    let mut i = 1;
    while i <= chroot.len() {
        let j = match chroot[i..].find('/') {
            Some(j) => j + i,
            None => chroot.len(),
        };
        let path = &chroot[..j];
        match client
            .create(path, Default::default(), CONTAINER_OPEN)
            .await
        {
            Ok(_) | Err(zk::Error::NodeExists) => {}
            Err(err) => {
                tokio::time::sleep(Duration::from_secs(30)).await;
                info!("connect err {:?} ,Try to re-establish the connection", err);
                return connect(cluster, chroot).await;
            }
        }
        i = j + 1;
    }
    match client.chroot(chroot.to_string()) {
        Ok(client) => client,
        Err(err) => {
            tokio::time::sleep(Duration::from_secs(30)).await;
            info!("connect err {:?} ,Try to re-establish the connection", err);
            return connect(cluster, chroot).await;
        }
    }
}

fn creat_resource_node(
    cluster: String,
    root: String,
    resource: Resource,
    map: Arc<RwLock<HashMap<String, Vec<Resource>>>>,
) {
    let mut path = root.to_string();
    let info = match &resource {
        Resource::Client(info) => {
            listener_resource_node_change(
                cluster.clone(),
                root,
                Resource::Client(info.clone()),
                map,
            );
            path.push_str(&("/".to_owned() + &info.server_name + "/consumers"));
            info
        }
        Resource::Server(info) => {
            path.push_str(&("/".to_owned() + &info.server_name + "/providers"));
            info
        }
    };
    let node_name = encode_url(&resource);
    let node_data = serde_json::to_string(&info).unwrap();
    tokio::spawn(async move {
        loop {
            let client = connect(&cluster, &path).await;
            match client
                .create(&node_name, node_data.as_bytes(), EPHEMERAL_OPEN)
                .await
            {
                Ok(_) => {}
                Err(err) => {
                    info!("create node err {:?}", err);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            }
            match client.check_and_watch_stat(&node_name).await {
                Ok(watch) => {
                    let event = watch.1.changed().await;
                    info!("resource node event {:?}", event);
                }
                Err(err) => {
                    info!("resource node err {:?}", err);
                }
            };
            drop(client);
        }
    });
}

fn listener_resource_node_change(
    cluster: String,
    root: String,
    resource: Resource,
    map: Arc<RwLock<HashMap<String, Vec<Resource>>>>,
) {
    let mut path = root;
    let info = match resource {
        Resource::Client(info) => {
            path.push_str(&("/".to_owned() + &info.server_name + "/providers"));
            info
        }
        Resource::Server(_) => return,
    };
    tokio::spawn(async move {
        let mut client = connect(&cluster.clone(), &path).await;
        let map = map;
        let info = info;
        loop {
            let watcher: (Vec<String>, zk::Stat, OneshotWatcher) =
                match client.get_and_watch_children("/").await {
                    Ok(watcher) => watcher,
                    Err(_) => {
                        client = connect(&cluster.clone(), &path).await;
                        continue;
                    }
                };
            let mut server_list = vec![];
            for node in watcher.0 {
                let resource = decode_url(&node);
                if let Ok(resource) = resource {
                    if let Resource::Server(resource_info) = resource {
                        if &info.version == &resource_info.version {
                            server_list.push(Resource::Server(resource_info));
                        }
                    }
                }
            }
            let mut key = info.server_name.clone();
            if let Some(version) = &info.version {
                key.push_str(":");
                key.push_str(version);
            }
            info!("update server cache {:?} : {:?}", key, server_list);
            let mut temp_map = map.write().await;
            temp_map.insert(key, server_list);
            drop(temp_map);
            let event: zk::WatchedEvent = watcher.2.changed().await;
            if let zk::EventType::NodeChildrenChanged = event.event_type {
                info!("Monitor node changes");
            } else {
                client = connect(&cluster.clone(), &path).await;
            }
        }
    });
}
