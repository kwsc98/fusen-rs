use std::{fs, sync::Arc};

use toml::get_toml_by_context;
use yaml::get_yaml_by_context;

use crate::error::BoxError;

pub mod toml;
pub mod yaml;

pub trait HotConfig {
    fn build_hot_config(
        &mut self,
        ident: Self,
        listener: tokio::sync::mpsc::Receiver<nacos_sdk::api::config::ConfigResponse>,
    ) -> Result<(), BoxError>;

    #[allow(async_fn_in_trait)]
    async fn get_hot_config(&self) -> Option<Arc<Self>>;
}

pub fn get_config_by_file<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, BoxError> {
    let contents =
        fs::read_to_string(path).unwrap_or_else(|_| panic!("read path erro : {:?}", path));
    let file_type: Vec<&str> = path.split('.').collect();
    match file_type[file_type.len() - 1].as_bytes() {
        b"toml" => get_toml_by_context(&contents),
        b"yaml" => get_yaml_by_context(&contents),
        file_type => Err(format!("not support {:?}", file_type).into()),
    }
}
