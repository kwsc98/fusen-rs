use std::{sync::Arc, time::Duration};

use fusen_common::{
    config::ConfigManager,
    fusen_common_procedural_macro::StrategyDebug,
    nacos::{NacosConfig, config::NacosConfiguration},
};

#[derive(serde::Deserialize, StrategyDebug)]
pub struct CloudConfig {
    pub config: String,
    #[strategy(limit = 2)]
    pub username: String,
    #[strategy(mask)]
    pub phone: String,
    #[strategy(ignore)]
    pub password: String,
}

#[tokio::main]
async fn main() {
    let config: CloudConfig = CloudConfig {
        config: "config".to_string(),
        username: "kwsc98".to_string(),
        phone: "18687987678".to_string(),
        password: "xxyynnzzjj@123".to_string(),
    };
    //字段过滤
    println!("{config:?}");
    //nacos热配置
    let config = NacosConfig {
        server_addr: "127.0.0.1:8848".to_string(),
        namespace: None,
        username: None,
        password: None,
    };
    let config = NacosConfiguration::init_nacos_configuration(Arc::new(config))
        .await
        .unwrap();
    //可直接导入nacos : examples/resource/nacos_config_export_20250928160704.zip
    let cloud_config: ConfigManager<CloudConfig> = config
        .get_config_manager("application-config1", "DEFAULT_GROUP")
        .await
        .unwrap();
    println!("{:?}", cloud_config.get_hot_config().await);
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        println!("{:?}", cloud_config.get_hot_config().await);
    }
}
