use crate::{ConfigDocument, ConfigError, ConfigErrorKind, ConfigFormat, ConfigOperation};
use serde::de::DeserializeOwned;
use std::path::Path;

/// Deserializes one configuration document according to its declared format.
pub fn parse<T: DeserializeOwned>(document: &ConfigDocument) -> Result<T, ConfigError> {
    match document.format() {
        ConfigFormat::Toml => parse_toml(document.content()),
        ConfigFormat::Yaml => parse_yaml(document.content()),
    }
}

/// Deserializes TOML text into a typed configuration value.
pub fn parse_toml<T: DeserializeOwned>(content: &str) -> Result<T, ConfigError> {
    toml::from_str(content).map_err(|error| {
        ConfigError::new(ConfigOperation::Parse, ConfigErrorKind::InvalidData, error)
    })
}

/// Deserializes YAML text when the `yaml` feature is enabled.
pub fn parse_yaml<T: DeserializeOwned>(content: &str) -> Result<T, ConfigError> {
    #[cfg(feature = "yaml")]
    {
        serde_yaml_ng::from_str(content).map_err(|error| {
            ConfigError::new(ConfigOperation::Parse, ConfigErrorKind::InvalidData, error)
        })
    }
    #[cfg(not(feature = "yaml"))]
    {
        let _ = content;
        Err(ConfigError::message(
            ConfigOperation::Parse,
            ConfigErrorKind::UnsupportedFormat,
            "YAML support requires the fusen-config `yaml` feature",
        ))
    }
}

/// Reads and deserializes a TOML or YAML file based on its extension.
pub fn load<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T, ConfigError> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path)
        .map_err(|error| ConfigError::new(ConfigOperation::Read, ConfigErrorKind::Io, error))?;
    let format = path
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(ConfigFormat::from_name)
        .ok_or_else(|| {
            ConfigError::message(
                ConfigOperation::Read,
                ConfigErrorKind::UnsupportedFormat,
                format!(
                    "unsupported configuration file extension for {}",
                    path.display()
                ),
            )
        })?;
    parse(&ConfigDocument::new(format, content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Demo {
        value: String,
    }

    #[test]
    fn parses_toml() {
        assert_eq!(
            parse_toml::<Demo>("value = 'ok'").unwrap(),
            Demo { value: "ok".into() }
        );
    }

    #[test]
    fn invalid_toml_is_classified() {
        let error = parse_toml::<Demo>("value = [").unwrap_err();
        assert_eq!(error.operation(), ConfigOperation::Parse);
        assert_eq!(error.kind(), ConfigErrorKind::InvalidData);
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn parses_yaml() {
        assert_eq!(
            parse_yaml::<Demo>("value: ok").unwrap(),
            Demo { value: "ok".into() }
        );
    }
}
