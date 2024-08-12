use serde_json::json;
use toml::Value;

use crate::error::BoxError;

pub fn get_toml_by_context< T: serde::de::DeserializeOwned>(toml_context: &str) -> Result<T, BoxError> {
    // 解析 TOML 文件内容
    let parsed_toml: Value = toml_context.parse()?;
    let json = json!(parsed_toml);
    Ok(T::deserialize(json).map_err(|e| format!("toml to json error {:?}", e))?)
}
