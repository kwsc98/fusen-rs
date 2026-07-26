//! Nacos last-good hot-configuration example.

use fusen_config::{ConfigKey, ConfigSource};
use fusen_nacos::{NacosConfig, NacosConfigSource};

#[derive(serde::Deserialize)]
struct CloudConfig {
    config: String,
    username: String,
    phone: String,
    password: String,
}

impl std::fmt::Debug for CloudConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudConfig")
            .field("config", &self.config)
            .field("username", &self.username)
            .field("phone", &redacted(&self.phone))
            .field("password", &redacted(&self.password))
            .finish()
    }
}

fn redacted(_value: &str) -> &'static str {
    "<redacted>"
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

    let config = NacosConfig::builder()
        .server_addr(std::env::var("NACOS_ADDR").unwrap_or_else(|_| "127.0.0.1:8848".to_owned()))
        .build();
    let source = NacosConfigSource::connect("fusen-nacos-hot-config", config).await?;
    // This archive can be imported directly into Nacos:
    // examples/resource/nacos_config_export_20250928160704.zip
    let key = ConfigKey::builder("application-config1")
        .group("DEFAULT_GROUP")
        .build()?;
    let handle = source.prepare(key)?;
    handle.activate().await?;
    let mut cloud_config = handle.typed::<CloudConfig>()?;
    println!("{:?}", cloud_config.current());
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            updated = cloud_config.changed() => {
                println!("{:?}", updated?.value());
            }
        }
    }
    cloud_config.close().await?;
    Ok(())
}
