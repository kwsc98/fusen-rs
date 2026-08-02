use crate::ContractError;
use std::collections::BTreeSet;

/// Stable identifier for the HTTP representation used by a service invocation.
pub const HTTP_JSON_V1: &str = "http-json-v1";

/// A validated service-invocation HTTP binding identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HttpBindingId(String);

impl HttpBindingId {
    /// Creates a binding identifier from its stable registry and telemetry representation.
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if valid_binding_id(&value) {
            Ok(Self(value))
        } else {
            Err(ContractError::InvalidHttpBinding)
        }
    }

    /// Returns the stable binding identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for HttpBindingId {
    fn default() -> Self {
        Self(HTTP_JSON_V1.to_owned())
    }
}

impl std::str::FromStr for HttpBindingId {
    type Err = ContractError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl std::fmt::Display for HttpBindingId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A validated non-empty set of supported HTTP transport versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HttpVersionSet(u8);

impl HttpVersionSet {
    const HTTP_1_1_BIT: u8 = 1 << 0;
    const HTTP_2_BIT: u8 = 1 << 1;

    /// Only HTTP/1.1 is supported.
    pub const HTTP_1_1: Self = Self(Self::HTTP_1_1_BIT);
    /// Only HTTP/2 is supported.
    pub const HTTP_2: Self = Self(Self::HTTP_2_BIT);
    /// HTTP/1.1 and HTTP/2 are both supported.
    pub const ALL: Self = Self(Self::HTTP_1_1_BIT | Self::HTTP_2_BIT);

    /// Builds a non-empty set from supported HTTP versions.
    pub fn new(versions: impl IntoIterator<Item = http::Version>) -> Result<Self, ContractError> {
        let mut bits = 0;
        for version in versions {
            bits |= Self::bit(version)?;
        }
        if bits == 0 {
            Err(ContractError::EmptyHttpVersionSet)
        } else {
            Ok(Self(bits))
        }
    }

    /// Returns whether this set includes `version`.
    pub fn contains(self, version: http::Version) -> bool {
        let bit = if version == http::Version::HTTP_11 {
            Self::HTTP_1_1_BIT
        } else if version == http::Version::HTTP_2 {
            Self::HTTP_2_BIT
        } else {
            0
        };
        self.0 & bit != 0
    }

    /// Iterates in stable HTTP/1.1, HTTP/2 order.
    pub fn iter(self) -> impl Iterator<Item = http::Version> {
        [http::Version::HTTP_11, http::Version::HTTP_2]
            .into_iter()
            .filter(move |version| self.contains(*version))
    }

    fn bit(version: http::Version) -> Result<u8, ContractError> {
        if version == http::Version::HTTP_11 {
            Ok(Self::HTTP_1_1_BIT)
        } else if version == http::Version::HTTP_2 {
            Ok(Self::HTTP_2_BIT)
        } else {
            Err(ContractError::UnsupportedHttpVersion(version))
        }
    }
}

impl Default for HttpVersionSet {
    fn default() -> Self {
        Self::HTTP_1_1
    }
}

/// Client transport preference, independent from the selected HTTP binding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HttpVersionPolicy {
    /// Negotiate the best transport supported by the endpoint.
    #[default]
    Auto,
    /// Require HTTP/1.1.
    Http1,
    /// Require HTTP/2 over TLS negotiation.
    Http2,
    /// Require cleartext HTTP/2 prior knowledge.
    H2c,
}

/// Transport and invocation features advertised by one HTTP endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointCapabilities {
    http_versions: HttpVersionSet,
    bindings: Vec<HttpBindingId>,
    invocation_controls: bool,
}

impl EndpointCapabilities {
    /// Creates validated endpoint capabilities.
    pub fn new(
        http_versions: HttpVersionSet,
        bindings: impl IntoIterator<Item = HttpBindingId>,
        invocation_controls: bool,
    ) -> Result<Self, ContractError> {
        let bindings = bindings.into_iter().collect::<BTreeSet<_>>();
        if bindings.is_empty() {
            return Err(ContractError::EmptyHttpBindings);
        }
        Ok(Self {
            http_versions,
            bindings: bindings.into_iter().collect(),
            invocation_controls,
        })
    }

    /// Returns supported HTTP transport versions.
    pub const fn http_versions(&self) -> HttpVersionSet {
        self.http_versions
    }

    /// Returns supported invocation binding identifiers in deterministic order.
    pub fn bindings(&self) -> &[HttpBindingId] {
        &self.bindings
    }

    /// Returns whether this endpoint supports `binding`.
    pub fn supports_binding(&self, binding: &HttpBindingId) -> bool {
        self.bindings.binary_search(binding).is_ok()
    }

    /// Returns whether Fusen timeout and attempt controls are supported.
    pub const fn invocation_controls(&self) -> bool {
        self.invocation_controls
    }
}

impl Default for EndpointCapabilities {
    fn default() -> Self {
        Self {
            http_versions: HttpVersionSet::HTTP_1_1,
            bindings: vec![HttpBindingId::default()],
            invocation_controls: false,
        }
    }
}

fn valid_binding_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.split(['-', '.']).all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_ids_are_bounded_lowercase_segments() {
        assert_eq!(HttpBindingId::default().as_str(), HTTP_JSON_V1);
        assert_eq!(
            HttpBindingId::new("vendor.http-v2").unwrap().as_str(),
            "vendor.http-v2"
        );
        for invalid in ["", "HTTP-JSON-V1", "http_json_v1", "-http", "http."] {
            assert!(HttpBindingId::new(invalid).is_err(), "{invalid}");
        }
        assert!(HttpBindingId::new("a".repeat(65)).is_err());
    }

    #[test]
    fn version_sets_are_non_empty_and_reject_unrelated_versions() {
        assert_eq!(
            HttpVersionSet::new([http::Version::HTTP_2, http::Version::HTTP_11]).unwrap(),
            HttpVersionSet::ALL
        );
        assert_eq!(
            HttpVersionSet::ALL.iter().collect::<Vec<_>>(),
            [http::Version::HTTP_11, http::Version::HTTP_2]
        );
        assert!(HttpVersionSet::new([]).is_err());
        assert!(HttpVersionSet::new([http::Version::HTTP_10]).is_err());
    }

    #[test]
    fn capabilities_are_deterministic_and_non_empty() {
        let json = HttpBindingId::default();
        let vendor = HttpBindingId::new("vendor-v1").unwrap();
        let capabilities = EndpointCapabilities::new(
            HttpVersionSet::ALL,
            [vendor.clone(), json.clone(), vendor],
            true,
        )
        .unwrap();
        assert_eq!(
            capabilities.bindings(),
            &[json.clone(), HttpBindingId::new("vendor-v1").unwrap()]
        );
        assert!(capabilities.supports_binding(&json));
        assert!(capabilities.invocation_controls());
        assert!(EndpointCapabilities::new(HttpVersionSet::HTTP_1_1, [], false).is_err());
    }
}
