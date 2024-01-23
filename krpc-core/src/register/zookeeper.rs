use hyper::client;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, watch, RwLock};
use uuid::uuid;
use zk::{AddWatchMode, Client, OneshotWatcher};

use crate::register::Info;

use super::{Register, RegisterInfo, Resource};
use zookeeper_client as zk;

static PERSISTENT_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Persistent.with_acls(zk::Acls::anyone_all());
static EPHEMERAL_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Ephemeral.with_acls(zk::Acls::anyone_all());
static EPHEMERAL_SEQ_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::EphemeralSequential.with_acls(zk::Acls::anyone_all());
static CONTAINER_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Container.with_acls(zk::Acls::anyone_all());

struct KrpcZookeeper {
    root_path: String,

    register_info: RegisterInfo,

    map: Arc<RwLock<HashMap<String, (Vec<Resource>, Vec<Resource>)>>>,

    monitors: Vec<Client>,
}

impl Register for KrpcZookeeper {
    async fn init(
        register_info: RegisterInfo,
        map: Arc<RwLock<HashMap<String, (Vec<Resource>, Vec<Resource>)>>>,
    ) -> Self {
        let root_path = "/krpc/".to_string() + &register_info.name_space;
        let krpc_zookeeper = KrpcZookeeper {
            root_path,
            register_info,
            map,
            monitors: vec![],
        };
        return krpc_zookeeper;
    }

    async fn add_resource(&mut self, resource: Resource) {
        let client: Client =
            creat_resource_node(&self.register_info.addr, &self.root_path, resource).await;
        self.monitors.push(client);
    }
}

#[tokio::test]
async fn test() {
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
        port: Some(8080),
    });

    let mut watch = listener_resource_node("127.0.0.1:2181", "/krpc/default", resource.clone()).await;
    tokio::spawn(async move {
        loop {
            let ds = watch.changed().await;
            println!("dsds{:?}",ds);
        }
    });

    zk.add_resource(resource).await;
    zk.add_resource(Resource::Server(Info {
        server_name: "TestServer".to_string(),
        version: "1.0.0".to_string(),
        ip: "127.0.0.2".to_string(),
        port: Some(8080),
    }))
    .await;

    zk.add_resource(Resource::Client(Info {
        server_name: "TestServer".to_string(),
        version: "1.0.0".to_string(),
        ip: "127.0.0.2".to_string(),
        port: None,
    }))
    .await;

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

async fn connect(cluster: &str, chroot: &str) -> zk::Client {
    let client = zk::Client::connect(cluster).await.unwrap();
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
            Err(err) => panic!("{err}"),
        }
        i = j + 1;
    }
    client.chroot(chroot).unwrap()
}

async fn creat_resource_node(cluster: &str, root: &str, resource: Resource) -> zk::Client {
    let mut path = root.to_string();
    let info = match resource {
        Resource::Client(info) => {
            path.push_str(&("/".to_owned() + &info.server_name + "/client/"));
            info
        }
        Resource::Server(info) => {
            path.push_str(&("/".to_owned() + &info.server_name + "/server/"));
            info
        }
    };
    path.push_str(&info.version);
    let client = connect(cluster, &path).await;
    let mut creat_type = EPHEMERAL_OPEN;
    let mut node_name = "/".to_owned() + &info.ip.clone();
    match info.port {
        Some(port) => node_name.push_str(&(":".to_owned() + &port.to_string())),
        None => {
            creat_type = EPHEMERAL_SEQ_OPEN;
            node_name.push_str(":");
        }
    };
    let node_data = serde_json::to_string(&info).unwrap();
    match client
        .create(&node_name, node_data.as_bytes(), creat_type)
        .await
    {
        Ok(_) | Err(zk::Error::NodeExists) => {}
        Err(err) => panic!("{err}"),
    }
    return client;
}

async fn listener_resource_node(cluster: &str, root: &str, resource: Resource) -> zk::PersistentWatcher {
    let mut path = root.to_string();
    let info = match resource {
        Resource::Client(info) => {
            path.push_str(&("/".to_owned() + &info.server_name + "/client/"));
            info
        }
        Resource::Server(info) => {
            path.push_str(&("/".to_owned() + &info.server_name + "/server/"));
            info
        }
    };
    path.push_str(&info.version);
    let client = connect(cluster, &path).await;
    let watcher = client.watch("/",AddWatchMode::PersistentRecursive).await.unwrap();
    return watcher;
}
