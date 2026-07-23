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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = CloudConfig {
        config: "config".to_string(),
        username: "kwsc98".to_string(),
        phone: "18687987678".to_string(),
        password: "xxyynnzzjj@123".to_string(),
    };
    println!("{config:?}");

    let config = NacosConfig {
        server_addr: std::env::var("NACOS_ADDR").unwrap_or_else(|_| "127.0.0.1:8848".to_owned()),
        namespace: None,
        username: None,
        password: None,
    };
    let config = NacosConfiguration::init_nacos_configuration(Arc::new(config)).await?;
    // This archive can be imported directly into Nacos:
    // examples/resource/nacos_config_export_20250928160704.zip
    let cloud_config: ConfigManager<CloudConfig> = config
        .get_config_manager("application-config1", "DEFAULT_GROUP")
        .await?;
    println!("{:?}", cloud_config.get_hot_config());
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                println!("{:?}", cloud_config.get_hot_config());
            }
        }
    }
    cloud_config.close().await?;
    Ok(())
}
