use async_recursion::async_recursion;
use krpc_common::get_uuid;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info};
use zk::OneshotWatcher;

use crate::register::Info;

use super::{Register, RegisterInfo, Resource};
use zookeeper_client as zk;

static EPHEMERAL_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Ephemeral.with_acls(zk::Acls::anyone_all());
static CONTAINER_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Container.with_acls(zk::Acls::anyone_all());

struct KrpcZookeeper {
    root_path: String,

    register_info: RegisterInfo,

    map: Arc<RwLock<HashMap<String, Vec<Info>>>>,
}

impl Register for KrpcZookeeper {
    async fn init(
        register_info: RegisterInfo,
        map: Arc<RwLock<HashMap<String, Vec<Info>>>>,
    ) -> Self {
        let root_path = "/krpc/".to_string() + &register_info.name_space;
        let krpc_zookeeper = KrpcZookeeper {
            root_path,
            register_info,
            map,
        };
        return krpc_zookeeper;
    }

    async fn add_resource(&mut self, resource: Resource) {
        creat_resource_node(
            self.register_info.addr.clone(),
            self.root_path.clone(),
            resource,
            self.map.clone(),
        )
        .await;
    }
}

#[tokio::test]
async fn test() {
    // krpc_common::init_log();
    let register_info = RegisterInfo {
        addr: format!("127.0.0.1:{}", "2181"),
        name_space: "default".to_string(),
        register_type: crate::register::RegisterType::ZooKeeper,
    };
    let map = Arc::new(RwLock::new(HashMap::new()));
    let mut zk = KrpcZookeeper::init(register_info, map).await;

    let resource = Resource::Server(Info {
        server_name: "TestServer".to_string(),
        version: "1.0.0".to_string(),
        ip: "127.0.0.1".to_string(),
        port: Some("8080".to_string()),
    });
    zk.add_resource(resource).await;
    zk.add_resource(Resource::Server(Info {
        server_name: "TestServer".to_string(),
        version: "1.0.0".to_string(),
        ip: "127.0.0.2".to_string(),
        port: Some("8080".to_string()),
    }))
    .await;
    let de = Resource::Client(Info {
        server_name: "TestServer".to_string(),
        version: "1.0.0".to_string(),
        ip: "127.0.0.2".to_string(),
        port: None,
    });
    listener_resource_node_change(
        "127.0.0.1:2181".to_string(),
        "/krpc/default".to_string(),
        de.clone(),
        Arc::new(RwLock::new(HashMap::new())),
    )
    .await;
    zk.add_resource(de).await;

    zk.add_resource(Resource::Client(Info {
        server_name: "TestServer".to_string(),
        version: "1.0.0".to_string(),
        ip: "127.0.0.2".to_string(),
        port: None,
    }))
    .await;
    let mut msp: (mpsc::Sender<i32>, mpsc::Receiver<i32>) = mpsc::channel(1);
    msp.1.recv().await;
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

async fn creat_resource_node(
    cluster: String,
    root: String,
    resource: Resource,
    map: Arc<RwLock<HashMap<String, Vec<Info>>>>,
) {
    let mut path = root.to_string();
    let info = match resource {
        Resource::Client(info) => {
            listener_resource_node_change(
                cluster.clone(),
                root,
                Resource::Client(info.clone()),
                map,
            )
            .await;
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
        }
    });
}

async fn listener_resource_node_change(
    cluster: String,
    root: String,
    resource: Resource,
    map: Arc<RwLock<HashMap<String, Vec<Info>>>>,
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
                let resource_info = Info {
                    server_name: info.server_name.clone(),
                    version: info.version.clone(),
                    ip: server_info[0].to_string(),
                    port: Some(server_info[1].to_string()),
                };
                server_list.push(resource_info);
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
