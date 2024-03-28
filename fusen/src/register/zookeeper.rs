use super::{Category, Register, Resource};
use crate::support::dubbo::{decode_url, encode_url};
use async_recursion::async_recursion;
use fusen_common::{server::Protocol, url::UrlConfig};
use fusen_macro::UrlConfig;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, f32::consts::E, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tracing::info;
use zk::{Client, OneshotWatcher};
use zookeeper_client as zk;

static EPHEMERAL_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Ephemeral.with_acls(zk::Acls::anyone_all());
static CONTAINER_OPEN: &zk::CreateOptions<'static> =
    &zk::CreateMode::Container.with_acls(zk::Acls::anyone_all());

pub struct FusenZookeeper {
    cluster: String,
}

#[derive(UrlConfig, Serialize, Deserialize)]
pub struct ZookeeperConfig {
    cluster: String,
}

impl FusenZookeeper {
    pub async fn init(url: &str) -> crate::Result<Self> {
        let config = ZookeeperConfig::from_url(url)?;
        Ok(Self {
            cluster: config.cluster.to_string(),
        })
    }
}

impl Register for FusenZookeeper {
    fn check(&self, protocol: &Vec<Protocol>) -> crate::Result<String> {
        for protocol in protocol {
            if let Protocol::HTTP2(port) = protocol {
                return Ok(port.clone());
            }
        }
        return Err("need monitor Http2".into());
    }

    fn register(&self, resource: Resource) -> fusen_common::FusenFuture<Result<(), crate::Error>> {
        let cluster = self.cluster.to_owned();
        Box::pin(async move { creat_resource_node(cluster, &resource).await })
    }

    fn subscribe(
        &self,
        resource: Resource,
    ) -> fusen_common::FusenFuture<Result<super::Directory, crate::Error>> {
        let cluster = self.cluster.to_owned();
        Box::pin(async move {
            creat_resource_node(cluster.clone(), &resource).await?;
            listener_resource_node_change(cluster, resource).await
        })
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

async fn creat_resource_node(cluster: String, resource: &Resource) -> crate::Result<()> {
    let mut path = "/dubbo".to_owned();
    match &resource.category {
        &Category::Client => {
            path.push_str(&("/".to_owned() + &resource.server_name + "/consumers"));
        }
        &Category::Server => {
            path.push_str(&("/".to_owned() + &resource.server_name + "/providers"));
        }
    };
    let node_name = encode_url(&resource);
    let node_data = serde_json::to_string(&resource).unwrap();
    let client = connect(&cluster, &path).await;
    match client
        .create(&node_name, node_data.as_bytes(), EPHEMERAL_OPEN)
        .await
    {
        Ok(_) => return Ok(()),
        Err(err) => {
            info!("create node err {:?}", err);
            return Err(Box::new(err));
        }
    }
}

async fn listener_resource_node_change(
    cluster: String,
    resource: Resource,
) -> Result<super::Directory, crate::Error> {
    let mut path = "/dubbo".to_owned();
    match &resource.category {
        &Category::Client => {
            path.push_str(&("/".to_owned() + &resource.server_name + "/providers"));
        }
        &Category::Server => return Err("server cloud be listener".into()),
    };
    let directory = super::Directory::new().await;
    let directory_clone = directory.clone();

    tokio::spawn(async move {
        let mut client = connect(&cluster.clone(), &path).await;
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
                    if let &Category::Server = &resource.category {
                        if &resource.version == &resource.version {
                            server_list.push(resource);
                        }
                    }
                }
            }
            let event: zk::WatchedEvent = watcher.2.changed().await;
            if let zk::EventType::NodeChildrenChanged = event.event_type {
                info!("Monitor node changes");
            } else {
                client = connect(&cluster.clone(), &path).await;
            }
        }
    });
    // tokio::spawn(async move {
    //     let mut client = connect(&cluster.clone(), &path).await;
    //     let info = resource;
    //     loop {
    //         let watcher: (Vec<String>, zk::Stat, OneshotWatcher) =
    //             match client.get_and_watch_children("/").await {
    //                 Ok(watcher) => watcher,
    //                 Err(_) => {
    //                     client = connect(&cluster.clone(), &path).await;
    //                     continue;
    //                 }
    //             };
    //         let mut server_list = vec![];
    //         for node in watcher.0 {
    //             let resource = decode_url(&node);
    //             if let Ok(resource) = resource {
    //                 if let &Category::Server = &resource.category {
    //                     if &info.version == &resource.version {
    //                         server_list.push(resource);
    //                     }
    //                 }
    //             }
    //         }
    //         let mut key = info.server_name.clone();
    //         if let Some(version) = &info.version {
    //             key.push_str(":");
    //             key.push_str(version);
    //         }
    //         info!("update server cache {:?} : {:?}", key, server_list);
    //         let mut temp_map = map.write().await;
    //         temp_map.insert(key, Arc::new(server_list));
    //         drop(temp_map);
    //         let event: zk::WatchedEvent = watcher.2.changed().await;
    //         if let zk::EventType::NodeChildrenChanged = event.event_type {
    //             info!("Monitor node changes");
    //         } else {
    //             client = connect(&cluster.clone(), &path).await;
    //         }
    //     }
    // }
    Ok(directory_clone)
}
