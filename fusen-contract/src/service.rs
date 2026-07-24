use http::Method;
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

/// Deterministically ordered provider metadata.
pub type Metadata = BTreeMap<String, String>;

/// Invalid service contract data.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
#[non_exhaustive]
pub enum ContractError {
    /// A required identifier is empty or surrounded by whitespace.
    #[error("{0} must be non-empty and must not contain surrounding whitespace")]
    InvalidIdentifier(&'static str),
    /// User metadata attempted to use an invalid or framework-reserved key.
    #[error("invalid metadata key {0:?}")]
    InvalidMetadataKey(String),
    /// A service endpoint is not a supported absolute HTTP(S) URL.
    #[error("invalid service endpoint: {0}")]
    InvalidEndpoint(String),
    /// A service weight is zero, negative, NaN, or infinite.
    #[error("service weight must be finite and greater than zero")]
    InvalidWeight,
    /// A service registration contains no callable methods.
    #[error("service registration must contain at least one method")]
    EmptyMethods,
    /// A method descriptor contains invalid HTTP or route metadata.
    #[error("invalid method descriptor: {0}")]
    InvalidMethod(String),
}

/// Identifies the service watched by one discovery subscription.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServiceSelector {
    service_id: String,
    group: Option<String>,
    version: Option<String>,
    metadata: Metadata,
}

/// Declaration-order identifier for one RPC method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MethodId(u16);

impl MethodId {
    /// Creates a method identifier from its declaration-order value.
    #[doc(hidden)]
    pub const fn __new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the declaration-order value.
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Returns the identifier as an index into a descriptor method slice.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl ServiceSelector {
    /// Creates a selector after validating its service identity.
    pub fn new(
        service_id: impl Into<String>,
        group: Option<String>,
        version: Option<String>,
    ) -> Result<Self, ContractError> {
        Ok(Self {
            service_id: validate_identifier(service_id.into(), "service id")?,
            group: validate_optional_identifier(group, "service group")?,
            version: validate_optional_identifier(version, "service version")?,
            metadata: Metadata::new(),
        })
    }

    /// Adds provider-specific user metadata.
    pub fn with_metadata(mut self, metadata: Metadata) -> Result<Self, ContractError> {
        validate_user_metadata(&metadata)?;
        self.metadata = metadata;
        Ok(self)
    }

    /// Returns the service identifier.
    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    /// Returns the optional service group.
    pub fn group(&self) -> Option<&str> {
        self.group.as_deref()
    }

    /// Returns the optional service version.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Returns provider-specific user metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

/// A validated absolute HTTP(S) service endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServiceEndpoint(Url);

impl ServiceEndpoint {
    /// Validates an already parsed URL.
    pub fn new(url: Url) -> Result<Self, ContractError> {
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.port_or_known_default().is_none()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ContractError::InvalidEndpoint(url.to_string()));
        }
        Ok(Self(url))
    }

    /// Returns the parsed endpoint URL.
    pub fn as_url(&self) -> &Url {
        &self.0
    }

    /// Returns the endpoint as a normalized URL string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::str::FromStr for ServiceEndpoint {
    type Err = ContractError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let url =
            Url::parse(value).map_err(|error| ContractError::InvalidEndpoint(error.to_string()))?;
        Self::new(url)
    }
}

impl TryFrom<Url> for ServiceEndpoint {
    type Error = ContractError;

    fn try_from(value: Url) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl std::fmt::Display for ServiceEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A finite positive service selection weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ServiceWeight(f64);

impl ServiceWeight {
    /// Creates a validated weight.
    pub fn new(value: f64) -> Result<Self, ContractError> {
        if value.is_finite() && value > 0.0 {
            Ok(Self(value))
        } else {
            Err(ContractError::InvalidWeight)
        }
    }

    /// Returns the numeric weight.
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Default for ServiceWeight {
    fn default() -> Self {
        Self(1.0)
    }
}

impl TryFrom<f64> for ServiceWeight {
    type Error = ContractError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// The HTTP location from which one generated RPC argument is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParameterSource {
    /// A named `{parameter}` segment in the route template.
    Path,
    /// A URL query parameter.
    Query,
    /// A JSON request body argument.
    Body,
}

/// Validated metadata for one RPC argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterDescriptor {
    name: String,
    source: ParameterSource,
}

impl ParameterDescriptor {
    /// Creates a parameter descriptor.
    #[doc(hidden)]
    pub fn __new(name: impl Into<String>, source: ParameterSource) -> Result<Self, ContractError> {
        Ok(Self {
            name: validate_identifier(name.into(), "parameter name")?,
            source,
        })
    }

    /// Returns the parameter name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the parameter source.
    pub const fn source(&self) -> ParameterSource {
        self.source
    }
}

/// Validated HTTP route metadata for one RPC method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDescriptor {
    id: MethodId,
    name: String,
    path: String,
    method: Method,
    parameters: Vec<ParameterDescriptor>,
}

impl MethodDescriptor {
    /// Creates and validates a method descriptor.
    #[doc(hidden)]
    pub fn __new(
        id: MethodId,
        name: impl Into<String>,
        method: Method,
        path: impl Into<String>,
        parameters: Vec<ParameterDescriptor>,
    ) -> Result<Self, ContractError> {
        let name = validate_identifier(name.into(), "method name")?;
        let path = path.into();
        validate_method(&method, &path, &parameters)?;
        Ok(Self {
            id,
            name,
            path,
            method,
            parameters,
        })
    }

    /// Returns the declaration-order method identifier.
    pub const fn id(&self) -> MethodId {
        self.id
    }

    /// Returns the generated method name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the route template.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the HTTP method.
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Returns ordered RPC parameter metadata.
    pub fn parameters(&self) -> &[ParameterDescriptor] {
        &self.parameters
    }
}

/// The single immutable description shared by generated clients, servers, and registries.
#[derive(Debug)]
pub struct ServiceDescriptor {
    selector: ServiceSelector,
    methods: Vec<MethodDescriptor>,
    identity: String,
}

impl ServiceDescriptor {
    /// Creates and validates one complete service contract.
    #[doc(hidden)]
    pub fn __new(
        service_id: impl Into<String>,
        version: Option<&str>,
        group: Option<&str>,
        methods: Vec<MethodDescriptor>,
    ) -> Result<Self, ContractError> {
        let selector = ServiceSelector::new(
            service_id,
            group.map(str::to_owned),
            version.map(str::to_owned),
        )?;
        Self::__from_selector(selector, methods)
    }

    /// Creates a service descriptor from an already validated selector.
    #[doc(hidden)]
    pub fn __from_selector(
        selector: ServiceSelector,
        methods: Vec<MethodDescriptor>,
    ) -> Result<Self, ContractError> {
        if methods.is_empty() {
            return Err(ContractError::EmptyMethods);
        }
        let mut method_names = BTreeSet::new();
        for (index, method) in methods.iter().enumerate() {
            if method.id().index() != index {
                return Err(ContractError::InvalidMethod(format!(
                    "method {} has non-contiguous id {}",
                    method.name(),
                    method.id().get()
                )));
            }
            if !method_names.insert(method.name()) {
                return Err(ContractError::InvalidMethod(format!(
                    "duplicate method {}",
                    method.name()
                )));
            }
        }
        let identity = format!(
            "{}:{:?}:{:?}",
            selector.service_id(),
            selector.version(),
            selector.group()
        );
        Ok(Self {
            selector,
            methods,
            identity,
        })
    }

    /// Returns the service selector used by discovery and registration.
    pub const fn selector(&self) -> &ServiceSelector {
        &self.selector
    }

    /// Returns the methods in declaration order.
    pub fn methods(&self) -> &[MethodDescriptor] {
        &self.methods
    }

    /// Returns a method by its declaration-order identifier.
    pub fn method(&self, id: MethodId) -> Option<&MethodDescriptor> {
        self.methods
            .get(id.index())
            .filter(|method| method.id() == id)
    }

    /// Returns a stable identity used for duplicate-service detection.
    pub fn identity(&self) -> &str {
        &self.identity
    }
}

/// A validated service registration submitted to a registry provider.
#[derive(Clone, Debug)]
pub struct ServiceRegistration {
    descriptor: &'static ServiceDescriptor,
    endpoint: ServiceEndpoint,
    weight: ServiceWeight,
}

impl ServiceRegistration {
    /// Creates a complete service registration.
    #[doc(hidden)]
    pub fn __new(
        descriptor: &'static ServiceDescriptor,
        endpoint: ServiceEndpoint,
        weight: ServiceWeight,
    ) -> Result<Self, ContractError> {
        Ok(Self {
            descriptor,
            endpoint,
            weight,
        })
    }

    /// Returns the shared service descriptor.
    pub const fn descriptor(&self) -> &'static ServiceDescriptor {
        self.descriptor
    }

    /// Returns the registered service selector.
    pub fn selector(&self) -> &ServiceSelector {
        self.descriptor.selector()
    }

    /// Returns the advertised endpoint.
    pub fn endpoint(&self) -> &ServiceEndpoint {
        &self.endpoint
    }

    /// Returns registered method metadata.
    pub fn methods(&self) -> &[MethodDescriptor] {
        self.descriptor.methods()
    }

    /// Returns the service weight.
    pub const fn weight(&self) -> ServiceWeight {
        self.weight
    }
}

/// One healthy discovered service instance.
#[derive(Clone, Debug)]
pub struct ServiceInstance {
    endpoint: ServiceEndpoint,
    weight: ServiceWeight,
    metadata: Metadata,
}

impl ServiceInstance {
    /// Creates a discovered service instance.
    pub fn new(endpoint: ServiceEndpoint, weight: ServiceWeight) -> Self {
        Self {
            endpoint,
            weight,
            metadata: Metadata::new(),
        }
    }

    /// Adds provider-owned instance metadata.
    pub fn with_metadata(mut self, metadata: Metadata) -> Result<Self, ContractError> {
        validate_metadata_keys(&metadata)?;
        self.metadata = metadata;
        Ok(self)
    }

    /// Returns the callable endpoint.
    pub fn endpoint(&self) -> &ServiceEndpoint {
        &self.endpoint
    }

    /// Returns the selection weight.
    pub const fn weight(&self) -> ServiceWeight {
        self.weight
    }

    /// Returns provider-owned metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

fn validate_identifier(value: String, field: &'static str) -> Result<String, ContractError> {
    if value.is_empty() || value.trim() != value {
        Err(ContractError::InvalidIdentifier(field))
    } else {
        Ok(value)
    }
}

fn validate_optional_identifier(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<String>, ContractError> {
    value
        .map(|value| validate_identifier(value, field))
        .transpose()
}

fn validate_metadata_keys(metadata: &Metadata) -> Result<(), ContractError> {
    if let Some(key) = metadata
        .keys()
        .find(|key| key.is_empty() || key.trim() != *key)
    {
        return Err(ContractError::InvalidMetadataKey(key.clone()));
    }
    Ok(())
}

fn validate_user_metadata(metadata: &Metadata) -> Result<(), ContractError> {
    validate_metadata_keys(metadata)?;
    if let Some(key) = metadata.keys().find(|key| key.starts_with("fusen.")) {
        return Err(ContractError::InvalidMetadataKey(key.clone()));
    }
    Ok(())
}

fn validate_method(
    method: &Method,
    path: &str,
    parameters: &[ParameterDescriptor],
) -> Result<(), ContractError> {
    if !matches!(
        *method,
        Method::GET
            | Method::POST
            | Method::PUT
            | Method::PATCH
            | Method::DELETE
            | Method::HEAD
            | Method::OPTIONS
    ) {
        return Err(ContractError::InvalidMethod(format!(
            "unsupported HTTP method {method}"
        )));
    }
    if !path.starts_with('/') || path.contains(['?', '#']) {
        return Err(ContractError::InvalidMethod(format!(
            "invalid route {path:?}"
        )));
    }

    let mut placeholders = BTreeSet::new();
    for segment in path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
    {
        if let Some(name) = segment
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            if name.is_empty() || name.contains(['{', '}']) || !placeholders.insert(name) {
                return Err(ContractError::InvalidMethod(
                    "route parameters must be non-empty, unique full segments".into(),
                ));
            }
        } else if segment.contains(['{', '}']) {
            return Err(ContractError::InvalidMethod(
                "route parameters must occupy full segments".into(),
            ));
        }
    }

    let query_method = matches!(*method, Method::GET | Method::DELETE | Method::HEAD);
    let mut names = BTreeSet::new();
    let mut path_parameters = BTreeSet::new();
    for parameter in parameters {
        if !names.insert(parameter.name()) {
            return Err(ContractError::InvalidMethod(format!(
                "duplicate parameter {}",
                parameter.name()
            )));
        }
        match parameter.source() {
            ParameterSource::Path => {
                path_parameters.insert(parameter.name());
            }
            ParameterSource::Query if !query_method => {
                return Err(ContractError::InvalidMethod(format!(
                    "parameter {} must use Body for {method}",
                    parameter.name()
                )));
            }
            ParameterSource::Body if query_method => {
                return Err(ContractError::InvalidMethod(format!(
                    "parameter {} must use Query for {method}",
                    parameter.name()
                )));
            }
            ParameterSource::Query | ParameterSource::Body => {}
        }
    }
    if placeholders != path_parameters {
        return Err(ContractError::InvalidMethod(
            "route placeholders do not match Path parameters".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_accepts_http_ipv6_and_base_paths() {
        let endpoint: ServiceEndpoint = "https://[::1]:8443/rpc".parse().unwrap();
        assert_eq!(endpoint.as_url().host_str(), Some("[::1]"));
        assert_eq!(endpoint.as_url().path(), "/rpc");
    }

    #[test]
    fn endpoint_rejects_unsupported_or_ambiguous_urls() {
        for value in [
            "ftp://localhost/rpc",
            "http://localhost/rpc?debug=true",
            "http://localhost/rpc#fragment",
            "/relative",
        ] {
            assert!(value.parse::<ServiceEndpoint>().is_err(), "{value}");
        }
    }

    #[test]
    fn weight_rejects_every_non_positive_or_non_finite_value() {
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(ServiceWeight::new(value).is_err());
        }
        assert_eq!(ServiceWeight::new(2.5).unwrap().get(), 2.5);
    }

    #[test]
    fn selector_rejects_invalid_identity_and_reserved_metadata() {
        assert!(ServiceSelector::new("", None, None).is_err());
        assert!(ServiceSelector::new(" demo", None, None).is_err());
        let metadata = Metadata::from([("fusen.scheme".into(), "https".into())]);
        assert!(
            ServiceSelector::new("demo", None, None)
                .unwrap()
                .with_metadata(metadata)
                .is_err()
        );
    }

    #[test]
    fn method_validates_parameter_sources_and_placeholders() {
        let valid = MethodDescriptor::__new(
            MethodId::__new(0),
            "find",
            Method::GET,
            "/users/{id}",
            vec![
                ParameterDescriptor::__new("id", ParameterSource::Path).unwrap(),
                ParameterDescriptor::__new("filter", ParameterSource::Query).unwrap(),
            ],
        );
        assert!(valid.is_ok());
        let invalid = MethodDescriptor::__new(
            MethodId::__new(0),
            "find",
            Method::GET,
            "/users/{id}",
            vec![ParameterDescriptor::__new("id", ParameterSource::Body).unwrap()],
        );
        assert!(invalid.is_err());
    }

    #[test]
    fn descriptor_rejects_duplicate_method_names_and_non_contiguous_ids() {
        let first = MethodDescriptor::__new(
            MethodId::__new(0),
            "find",
            Method::GET,
            "/users",
            Vec::new(),
        )
        .unwrap();
        let duplicate = MethodDescriptor::__new(
            MethodId::__new(1),
            "find",
            Method::GET,
            "/users/duplicate",
            Vec::new(),
        )
        .unwrap();
        assert!(ServiceDescriptor::__new("users", None, None, vec![first, duplicate]).is_err());

        let skipped = MethodDescriptor::__new(
            MethodId::__new(1),
            "find",
            Method::GET,
            "/users",
            Vec::new(),
        )
        .unwrap();
        assert!(ServiceDescriptor::__new("users", None, None, vec![skipped]).is_err());
    }
}
