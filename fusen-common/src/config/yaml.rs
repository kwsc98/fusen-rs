use crate::error::BoxError;
use serde_yaml::Value;

pub fn get_yaml_by_context< T: serde::de::DeserializeOwned>(
    yaml_context: &str,
) -> Result<T, BoxError> {
    // 解析 yaml 文件内容
    let parsed_toml: Value = serde_yaml::from_str(yaml_context)?;
    Ok(T::deserialize(parsed_toml).map_err(|e| format!("json to json error {:?}", e))?)
}
