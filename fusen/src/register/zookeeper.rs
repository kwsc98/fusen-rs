use super::{Category, Register, Resource, Type};
use crate::support::dubbo::{decode_url, encode_url};
use async_recursion::async_recursion;
use fusen_common::url::UrlConfig;
use fusen_macro::url_config;
use std::{sync::Arc, time::Duration};
use tracing::{debug, error, info};
use zk::OneshotWatcher;
use zookeeper_client as zk;

static EPHEMERAL_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Ephemeral.with_acls(zk::Acls::anyone_all());
static CONTAINER_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Container.with_acls(zk::Acls::anyone_all());

pub struct FusenZookeeper {
    config: Arc<ZookeeperConfig>,
    root_path: String,
}

#[url_config(attr = register)]
pub struct ZookeeperConfig {
    cluster: String,
    server_type: Type,
}

impl FusenZookeeper {
    pub fn init(url: &str) -> crate::Result<Self> {
        let config = ZookeeperConfig::from_url(url)?;
        let path = match config.server_type {
            Type::Dubbo => "/dubbo",
            Type::Fusen => "/fusen",
            Type::SpringCloud => return Err("zookeeper not support SpringCloud".into()),
        }
        .to_owned();
        Ok(Self {
            config: Arc::new(config),
            root_path: path,
        })
    }
}

impl Register for FusenZookeeper {
    fn register(&self, resource: Resource) -> fusen_common::FusenFuture<Result<(), crate::Error>> {
        let cluster = self.config.cluster.clone();
        let path = self.root_path.clone();
        Box::pin(async move { creat_resource_node(cluster, path, &resource).await })
    }

    fn subscribe(
        &self,
        resource: Resource,
    ) -> fusen_common::FusenFuture<Result<super::Directory, crate::Error>> {
        let cluster = self.config.cluster.clone();
        let path = self.root_path.clone();
        let server_type = self.config.server_type.clone();
        Box::pin(async move {
            creat_resource_node(cluster.clone(), path.clone(), &resource).await?;
            listener_resource_node_change(cluster, path, server_type, resource).await
        })
    }

    fn get_type(&self) -> Type {
        self.config.server_type.clone()
    }
}

#[async_recursion]
async fn connect(cluster: &str, chroot: &str) -> zk::Client {
    let client = match zk::Client::connect(cluster).await {
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
    mut path: String,
    resource: &Resource,
) -> crate::Result<()> {
    match &resource.category {
        Category::Client => {
            path.push_str(&("/".to_owned() + &resource.server_name + "/consumers"));
        }
        Category::Service => {
            path.push_str(&("/".to_owned() + &resource.server_name + "/providers"));
        }
        Category::Server => (),
    };
    let node_name = encode_url(resource);
    let node_data = serde_json::to_string(&resource).unwrap();
    let client = connect(&cluster, &path).await;
    match client
        .create(&node_name, node_data.as_bytes(), EPHEMERAL_OPEN)
        .await
    {
        Ok(_) => {
            debug!("node create success");
        }
        Err(err) => {
            if let zk::Error::NodeExists = err {
                debug!("node exists success")
            } else {
                info!("create node err {:?}", err);
                return Err(Box::new(err));
            }
        }
    };
    tokio::spawn(async move {
        let mut client = client;
        loop {
            match client.check_and_watch_stat(&node_name).await {
                Ok(watch) => {
                    let event = watch.1.changed().await;
                    info!("resource node event {:?}", event);
                }
                Err(err) => {
                    info!("resource node err {:?}", err);
                }
            };
            while let Err(err) = client
                .create(&node_name, node_data.as_bytes(), EPHEMERAL_OPEN)
                .await
            {
                error!("node : {:?} create err : {:?}", node_data, err);
                client = connect(&cluster, &path).await;
            }
        }
    });
    Ok(())
}

async fn listener_resource_node_change(
    cluster: String,
    mut path: String,
    server_type: Type,
    resource: Resource,
) -> Result<super::Directory, crate::Error> {
    let version = resource.version;
    match &resource.category {
        Category::Client => {
            path.push_str(&("/".to_owned() + &resource.server_name + "/providers"));
        }
        Category::Service => return Err("service cant be listener".into()),
        Category::Server => return Err("server cant be listener".into()),
    };
    let directory = super::Directory::new(Arc::new(server_type)).await;
    let directory_clone = directory.clone();
    let client = connect(&cluster.clone(), &path).await;
    let watcher: (Vec<String>, zk::Stat, OneshotWatcher) =
        match client.get_and_watch_children("/").await {
            Ok(watcher) => watcher,
            Err(err) => return Err(Box::new(err)),
        };
    let mut server_list = vec![];
    for node in watcher.0 {
        let resource_tmp = decode_url(&node);
        if let Ok(resource_tmp) = resource_tmp {
            if let &Category::Server = &resource_tmp.category {
                if version == resource_tmp.version {
                    server_list.push(resource_tmp);
                }
            }
        }
    }
    let _ = directory.change(server_list).await;
    tokio::spawn(async move {
        let mut client = connect(&cluster.clone(), &path).await;
        loop {
            let watcher = match client.get_and_watch_children("/").await {
                Ok(watcher) => watcher,
                Err(_) => {
                    client = connect(&cluster.clone(), &path).await;
                    continue;
                }
            };
            let mut server_list = vec![];
            for node in watcher.0 {
                let resource_tmp = decode_url(&node);
                if let Ok(resource_tmp) = resource_tmp {
                    if let &Category::Server = &resource_tmp.category {
                        if version == resource_tmp.version {
                            server_list.push(resource_tmp);
                        }
                    }
                }
            }
            let _ = directory.change(server_list).await;
            let event: zk::WatchedEvent = watcher.2.changed().await;
            if let zk::EventType::NodeChildrenChanged = event.event_type {
                info!("Monitor node changes");
            } else {
                client = connect(&cluster.clone(), &path).await;
            }
        }
    });
    Ok(directory_clone)
}
