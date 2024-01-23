use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, RwLock};

use super::{Register, RegisterInfo, Resource};
use zookeeper_client as zk;

static PERSISTENT_OPEN: &zk::CreateOptions<'static> = &zk::CreateMode::Persistent.with_acls(zk::Acls::anyone_all());
static EPHEMERAL_OPEN: &zk::CreateOptions<'static> = &zk::CreateMode::Ephemeral.with_acls(zk::Acls::anyone_all());
static CONTAINER_OPEN: &zk::CreateOptions<'static> = &zk::CreateMode::Container.with_acls(zk::Acls::anyone_all());



struct KrpcZookeeper {

    register_info: RegisterInfo,

    map: Arc<RwLock<HashMap<String, (Vec<Resource>, Vec<Resource>)>>>,
}

impl Register for KrpcZookeeper {
    async fn init(
        register_info: RegisterInfo,
        map: Arc<RwLock<HashMap<String, (Vec<Resource>, Vec<Resource>)>>>,
    ) -> Self {
        let krpc_zookeeper = KrpcZookeeper { register_info, map };
        let cluster = krpc_zookeeper.register_info.addr.clone();
        let root = "/krpc/".to_string() + &krpc_zookeeper.register_info.name_space;
        let client = connect(&cluster, &root[..]).await;
        
        return krpc_zookeeper;
    }

    async fn add_resource(&self, resource: Resource) {
        todo!()
    }
}


#[tokio::test]
async fn test() {
    let cluster = format!("127.0.0.1:{}", "2181");
    let client = connect(&cluster, "/dehudhueh").await;
    let (stat, _) = client.create("/krpc", "dsd1".as_bytes(), CONTAINER_OPEN).await.unwrap();
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
        match client.create(path, Default::default(), PERSISTENT_OPEN).await {
            Ok(_) | Err(zk::Error::NodeExists) => {},
            Err(err) => panic!("{err}"),
        }
        i = j + 1;
    }
    client.chroot(chroot).unwrap()
}