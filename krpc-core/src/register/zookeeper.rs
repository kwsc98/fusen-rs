use super::{Register, Resource, SocketInfo};
use crate::register::Info;
use async_recursion::async_recursion;
use krpc_common::get_uuid;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tracing::info;
use zk::OneshotWatcher;
use zookeeper_client as zk;

static EPHEMERAL_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Ephemeral.with_acls(zk::Acls::anyone_all());
static CONTAINER_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Container.with_acls(zk::Acls::anyone_all());

pub struct KrpcZookeeper {
    addr: String,

    root_path: String,

    map: Arc<RwLock<HashMap<String, Vec<SocketInfo>>>>,
}

impl Register for KrpcZookeeper {
    fn add_resource(&self, resource: Resource) {
        creat_resource_node(
            self.addr.clone(),
            self.root_path.clone(),
            resource,
            self.map.clone(),
        )
    }
}

impl KrpcZookeeper {
    pub fn init(
        addr: &str,
        name_space: &str,
        map: Arc<RwLock<HashMap<String, Vec<SocketInfo>>>>,
    ) -> Self {
        let root_path = "/krpc/".to_string() + name_space;
        let krpc_zookeeper = KrpcZookeeper {
            addr: addr.to_string(),
            root_path,
            map,
        };
        return krpc_zookeeper;
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
    map: Arc<RwLock<HashMap<String, Vec<SocketInfo>>>>,
) {
    let mut path = root.to_string();
    let info = match resource {
        Resource::Client(info) => {
            listener_resource_node_change(
                cluster.clone(),
                root,
                Resource::Client(info.clone()),
                map,
            );
            path.push_str(&("/".to_owned() + &info.server_name + "/client/"));
            info
        }
        Resource::Server(info) => {
            path.push_str(&("/".to_owned() + &info.server_name + "/server/"));
            info
        }
    };
    path.push_str(&info.version);
    let mut node_name = "/".to_owned() + &info.ip.clone();
    match info.port.clone() {
        Some(port) => node_name.push_str(&(":".to_owned() + &port.to_string())),
        None => {
            node_name.push_str(&(":".to_owned() + &get_uuid()));
        }
    };
    let node_data = serde_json::to_string(&info).unwrap();
    tokio::spawn(async move {
        loop {
            let client = connect(&cluster, &path).await;
            match client
                .create(&node_name, node_data.as_bytes(), EPHEMERAL_OPEN)
                .await
            {
                Ok(_) => {}
                Err(_err) => {
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
    map: Arc<RwLock<HashMap<String, Vec<SocketInfo>>>>,
) {
    let mut path = root;
    let info = match resource {
        Resource::Client(info) => {
            path.push_str(&("/".to_owned() + &info.server_name + "/server/"));
            info
        }
        Resource::Server(_) => return,
    };
    path.push_str(&info.version);
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
                let server_info: Vec<&str> = node.split(":").collect();
                let info = Info {
                    server_name: info.server_name.clone(),
                    version: info.version.clone(),
                    ip: server_info[0].to_string(),
                    port: Some(server_info[1].to_string()),
                };
                server_list.push(SocketInfo { info, sender: Arc::new(RwLock::new(None))});
            }
            let key = info.server_name.clone() + ":" + &info.version.clone();
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
