use crate::{ProtocolSet, WireProtocol};
use http::Method;
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

const MAX_IDENTITY_BYTES: usize = 128;

/// Deterministically ordered provider metadata.
pub type Metadata = BTreeMap<String, String>;

/// Invalid protocol or service contract data.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContractError {
    /// A required identity component is not a bounded ASCII token.
    #[error("invalid {0}: expected 1-128 ASCII letters, digits, '.', '_' or '-'")]
    InvalidIdentifier(&'static str),
    /// A provider instance identity is not a bounded stable token.
    #[error("invalid instance id: expected 1-128 ASCII letters, digits, '.', '_', '-' or ':'")]
    InvalidInstanceId,
    /// User metadata attempted to use an invalid or framework-reserved key.
    #[error("invalid metadata key {0:?}")]
    InvalidMetadataKey(String),
    /// A service endpoint is not a canonical absolute HTTP or HTTPS URL.
    #[error("invalid service endpoint: {0}")]
    InvalidEndpoint(String),
    /// A service weight is zero, negative, NaN, or infinite.
    #[error("service weight must be finite and greater than zero")]
    InvalidWeight,
    /// A protocol set contained no protocols.
    #[error("protocol set must contain at least one protocol")]
    EmptyProtocolSet,
    /// A service descriptor contains no callable methods.
    #[error("service descriptor must contain at least one method")]
    EmptyMethods,
    /// A method descriptor contains invalid Fusen or Spring Cloud metadata.
    #[error("invalid method descriptor: {0}")]
    InvalidMethod(String),
    /// A registration advertised a protocol not implemented by every service method.
    #[error("service {service} does not implement advertised protocol {protocol}")]
    UnsupportedServiceProtocol {
        /// Stable service identity.
        service: String,
        /// Protocol that cannot dispatch every method.
        protocol: WireProtocol,
    },
}

/// A caller-supplied stable identity for one provider instance.
///
/// The ID must remain unchanged while the same logical provider is re-registered. Endpoint
/// changes therefore do not appear as an unrelated instance to discovery consumers.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstanceId(String);

impl InstanceId {
    /// Creates a validated provider instance identity.
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if is_instance_id(&value) {
            Ok(Self(value))
        } else {
            Err(ContractError::InvalidInstanceId)
        }
    }

    /// Returns the stable identity string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for InstanceId {
    type Err = ContractError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl std::fmt::Display for InstanceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Identifies the service watched by one discovery subscription.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServiceSelector {
    service_id: String,
    group: Option<String>,
    version: Option<String>,
    metadata: Metadata,
    identity: String,
}

impl ServiceSelector {
    /// Creates a selector from validated service identity components.
    pub fn new(
        service_id: impl Into<String>,
        group: Option<String>,
        version: Option<String>,
    ) -> Result<Self, ContractError> {
        let service_id = validate_identifier(service_id.into(), "service id")?;
        let group = validate_optional_identifier(group, "service group")?;
        let version = validate_optional_identifier(version, "service version")?;
        let identity = service_identity(&service_id, group.as_deref(), version.as_deref());
        Ok(Self {
            service_id,
            group,
            version,
            metadata: Metadata::new(),
            identity,
        })
    }

    /// Adds provider-independent selector metadata.
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

    /// Returns provider-independent selector metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns the canonical `service[/group][@version]` identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }
}

/// Declaration-order identifier used for process-local O(1) method dispatch.
///
/// This value is not a wire identity. [`MethodDescriptor::fusen_identity`] remains stable when
/// declaration order changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MethodId(u16);

impl MethodId {
    /// Creates a declaration-order method identifier.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the declaration-order value.
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Returns the identifier as a descriptor slice index.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A canonical absolute HTTP or HTTPS service endpoint.
///
/// Parsing uses [`url::Url`], so host casing, default ports, percent encoding, and dot segments use
/// that type's canonical representation. HTTPS endpoints use client-side TLS; accepting one as a
/// server advertisement does not make the built-in server listener terminate TLS.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServiceEndpoint(Url);

impl ServiceEndpoint {
    /// Validates an already parsed canonical URL.
    pub fn new(url: Url) -> Result<Self, ContractError> {
        let valid = matches!(url.scheme(), "http" | "https")
            && !url.cannot_be_a_base()
            && url.host_str().is_some()
            && url.port_or_known_default().is_some_and(|port| port != 0)
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none();
        if valid {
            Ok(Self(url))
        } else {
            Err(ContractError::InvalidEndpoint(url.to_string()))
        }
    }

    /// Returns the canonical parsed endpoint.
    pub fn as_url(&self) -> &Url {
        &self.0
    }

    /// Returns the canonical endpoint string.
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

/// The explicit Spring Cloud HTTP location of one ordered RPC argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SpringCloudParameterSource {
    /// A named `{parameter}` path segment.
    Path,
    /// A URL query parameter.
    Query,
    /// A named field in a synthesized JSON request body object.
    BodyField,
    /// The complete JSON request body. It cannot be combined with body fields.
    Body,
}

/// The number of values represented by one Spring Cloud V1 parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SpringCloudParameterCardinality {
    /// At most one scalar value is represented on the wire.
    Scalar,
    /// Zero or more query values are represented as repeated keys.
    Repeated,
}

/// Spring Cloud V1 wire metadata for one ordered RPC argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpringCloudParameter {
    name: String,
    source: SpringCloudParameterSource,
    cardinality: SpringCloudParameterCardinality,
}

impl SpringCloudParameter {
    /// Creates validated parameter metadata.
    pub fn new(
        name: impl Into<String>,
        source: SpringCloudParameterSource,
        cardinality: SpringCloudParameterCardinality,
    ) -> Result<Self, ContractError> {
        Ok(Self {
            name: validate_identifier(name.into(), "Spring Cloud parameter name")?,
            source,
            cardinality,
        })
    }

    /// Returns the wire parameter name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the explicit HTTP argument location.
    pub const fn source(&self) -> SpringCloudParameterSource {
        self.source
    }

    /// Returns whether the wire parameter is scalar or repeated.
    pub const fn cardinality(&self) -> SpringCloudParameterCardinality {
        self.cardinality
    }
}

/// Optional Spring Cloud V1 HTTP mapping for one RPC method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpringCloudMethod {
    method: Method,
    path: String,
    parameters: Vec<SpringCloudParameter>,
}

impl SpringCloudMethod {
    /// Creates and validates an HTTP method, route template, and ordered parameter mapping.
    pub fn new(
        method: Method,
        path: impl Into<String>,
        parameters: Vec<SpringCloudParameter>,
    ) -> Result<Self, ContractError> {
        let path = path.into();
        validate_spring_cloud_method(&method, &path, &parameters)?;
        Ok(Self {
            method,
            path,
            parameters,
        })
    }

    /// Returns the HTTP method.
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Returns the absolute route template.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns mappings in generated RPC argument order.
    pub fn parameters(&self) -> &[SpringCloudParameter] {
        &self.parameters
    }
}

/// Versioned wire metadata for one generated RPC method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDescriptor {
    id: MethodId,
    fusen_identity: String,
    spring_cloud: Option<SpringCloudMethod>,
}

impl MethodDescriptor {
    /// Creates a method descriptor with an optional Spring Cloud V1 mapping.
    pub fn new(
        id: MethodId,
        fusen_identity: impl Into<String>,
        spring_cloud: Option<SpringCloudMethod>,
    ) -> Result<Self, ContractError> {
        let fusen_identity = validate_identifier(fusen_identity.into(), "Fusen method identity")?;
        Ok(Self {
            id,
            fusen_identity,
            spring_cloud,
        })
    }

    /// Returns the process-local declaration-order identifier.
    pub const fn id(&self) -> MethodId {
        self.id
    }

    /// Returns the stable Fusen V1 wire identity.
    pub fn fusen_identity(&self) -> &str {
        &self.fusen_identity
    }

    /// Returns whether the standard HTTP method permits automatic replay.
    ///
    /// Methods without an HTTP mapping and mappings using POST or PATCH are never replayed.
    pub fn allows_retries(&self) -> bool {
        self.spring_cloud.as_ref().is_some_and(|mapping| {
            matches!(
                *mapping.method(),
                Method::GET | Method::HEAD | Method::OPTIONS | Method::PUT | Method::DELETE
            )
        })
    }

    /// Returns the optional Spring Cloud V1 mapping.
    pub const fn spring_cloud(&self) -> Option<&SpringCloudMethod> {
        self.spring_cloud.as_ref()
    }
}

/// The immutable service description shared by generated clients, servers, and registries.
#[derive(Debug)]
pub struct ServiceDescriptor {
    selector: ServiceSelector,
    methods: Vec<MethodDescriptor>,
}

impl ServiceDescriptor {
    /// Creates a complete service contract.
    pub fn new(
        selector: ServiceSelector,
        methods: Vec<MethodDescriptor>,
    ) -> Result<Self, ContractError> {
        validate_methods(&methods)?;
        Ok(Self { selector, methods })
    }

    /// Returns the discovery and registration selector.
    pub const fn selector(&self) -> &ServiceSelector {
        &self.selector
    }

    /// Returns methods in declaration order.
    pub fn methods(&self) -> &[MethodDescriptor] {
        &self.methods
    }

    /// Returns a method by its process-local identifier.
    pub fn method(&self, id: MethodId) -> Option<&MethodDescriptor> {
        self.methods
            .get(id.index())
            .filter(|method| method.id() == id)
    }

    /// Returns the selector's stable service identity.
    pub fn identity(&self) -> &str {
        self.selector.identity()
    }

    /// Returns protocols implemented by every method in this service.
    pub fn supported_protocols(&self) -> ProtocolSet {
        if self
            .methods
            .iter()
            .all(|method| method.spring_cloud().is_some())
        {
            ProtocolSet::ALL
        } else {
            ProtocolSet::FUSEN_V1
        }
    }
}

/// A complete provider registration submitted to a registry.
#[derive(Clone, Debug)]
pub struct ServiceRegistration {
    instance_id: InstanceId,
    descriptor: &'static ServiceDescriptor,
    endpoint: ServiceEndpoint,
    protocols: ProtocolSet,
    weight: ServiceWeight,
}

impl ServiceRegistration {
    /// Creates a registration from already validated contract components.
    pub fn new(
        instance_id: InstanceId,
        descriptor: &'static ServiceDescriptor,
        endpoint: ServiceEndpoint,
        protocols: ProtocolSet,
        weight: ServiceWeight,
    ) -> Result<Self, ContractError> {
        if let Some(protocol) = protocols
            .iter()
            .find(|protocol| !descriptor.supported_protocols().contains(*protocol))
        {
            return Err(ContractError::UnsupportedServiceProtocol {
                service: descriptor.identity().to_owned(),
                protocol,
            });
        }
        Ok(Self {
            instance_id,
            descriptor,
            endpoint,
            protocols,
            weight,
        })
    }

    /// Returns the stable provider instance identity.
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    /// Returns the shared service descriptor.
    pub const fn descriptor(&self) -> &'static ServiceDescriptor {
        self.descriptor
    }

    /// Returns the registered service selector.
    pub fn selector(&self) -> &ServiceSelector {
        self.descriptor.selector()
    }

    /// Returns the advertised HTTP or HTTPS endpoint.
    pub const fn endpoint(&self) -> &ServiceEndpoint {
        &self.endpoint
    }

    /// Returns the advertised wire protocols.
    pub const fn protocols(&self) -> ProtocolSet {
        self.protocols
    }

    /// Returns the service weight.
    pub const fn weight(&self) -> ServiceWeight {
        self.weight
    }
}

/// One healthy discovered provider instance.
#[derive(Clone, Debug)]
pub struct ServiceInstance {
    instance_id: InstanceId,
    endpoint: ServiceEndpoint,
    weight: ServiceWeight,
    metadata: Metadata,
}

impl ServiceInstance {
    /// Creates a discovered instance from validated identity and endpoint values.
    pub fn new(instance_id: InstanceId, endpoint: ServiceEndpoint, weight: ServiceWeight) -> Self {
        Self {
            instance_id,
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

    /// Returns the stable provider identity.
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    /// Returns the callable HTTP or HTTPS endpoint.
    pub const fn endpoint(&self) -> &ServiceEndpoint {
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
    if is_identity_component(&value) {
        Ok(value)
    } else {
        Err(ContractError::InvalidIdentifier(field))
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

fn is_identity_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTITY_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_instance_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTITY_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn service_identity(service_id: &str, group: Option<&str>, version: Option<&str>) -> String {
    let mut identity = service_id.to_owned();
    if let Some(group) = group {
        identity.push('/');
        identity.push_str(group);
    }
    if let Some(version) = version {
        identity.push('@');
        identity.push_str(version);
    }
    identity
}

fn validate_metadata_keys(metadata: &Metadata) -> Result<(), ContractError> {
    if let Some(key) = metadata.keys().find(|key| {
        key.is_empty() || key.trim() != *key || key.bytes().any(|byte| byte.is_ascii_control())
    }) {
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

fn validate_spring_cloud_method(
    method: &Method,
    path: &str,
    parameters: &[SpringCloudParameter],
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
            "unsupported Spring Cloud HTTP method {method}"
        )));
    }
    if !path.starts_with('/')
        || path.contains(['?', '#'])
        || path.contains("//")
        || (path.len() > 1 && path.ends_with('/'))
    {
        return Err(ContractError::InvalidMethod(format!(
            "invalid Spring Cloud route {path:?}"
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
            if !is_identity_component(name) || !placeholders.insert(name) {
                return Err(ContractError::InvalidMethod(
                    "Spring Cloud path parameters must be unique full token segments".into(),
                ));
            }
        } else if segment.contains(['{', '}']) {
            return Err(ContractError::InvalidMethod(
                "Spring Cloud path parameters must occupy full segments".into(),
            ));
        }
    }

    let mut names = BTreeSet::new();
    let mut path_parameters = BTreeSet::new();
    let mut body_count = 0;
    let mut body_field_count = 0;
    for parameter in parameters {
        if !names.insert(parameter.name()) {
            return Err(ContractError::InvalidMethod(format!(
                "duplicate Spring Cloud parameter {}",
                parameter.name()
            )));
        }
        match parameter.source() {
            SpringCloudParameterSource::Path => {
                if parameter.cardinality() == SpringCloudParameterCardinality::Repeated {
                    return Err(ContractError::InvalidMethod(
                        "Spring Cloud repeated parameters may use only the Query source".into(),
                    ));
                }
                path_parameters.insert(parameter.name());
            }
            SpringCloudParameterSource::Body => {
                if parameter.cardinality() == SpringCloudParameterCardinality::Repeated {
                    return Err(ContractError::InvalidMethod(
                        "Spring Cloud repeated parameters may use only the Query source".into(),
                    ));
                }
                body_count += 1;
            }
            SpringCloudParameterSource::BodyField => {
                if parameter.cardinality() == SpringCloudParameterCardinality::Repeated {
                    return Err(ContractError::InvalidMethod(
                        "Spring Cloud repeated parameters may use only the Query source".into(),
                    ));
                }
                body_field_count += 1;
            }
            SpringCloudParameterSource::Query => {}
        }
    }
    if body_count > 1 {
        return Err(ContractError::InvalidMethod(
            "Spring Cloud V1 permits at most one body parameter".into(),
        ));
    }
    if body_count == 1 && body_field_count != 0 {
        return Err(ContractError::InvalidMethod(
            "a complete JSON body cannot be combined with synthesized body fields".into(),
        ));
    }
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
        && body_count + body_field_count != 0
    {
        return Err(ContractError::InvalidMethod(format!(
            "HTTP {method} methods do not accept a JSON request body"
        )));
    }
    if placeholders != path_parameters {
        return Err(ContractError::InvalidMethod(
            "Spring Cloud route placeholders do not match Path parameters".into(),
        ));
    }
    Ok(())
}

fn validate_methods(methods: &[MethodDescriptor]) -> Result<(), ContractError> {
    if methods.is_empty() {
        return Err(ContractError::EmptyMethods);
    }
    let mut fusen_identities = BTreeSet::new();
    let mut spring_routes = BTreeSet::new();
    for (index, method) in methods.iter().enumerate() {
        if method.id().index() != index {
            return Err(ContractError::InvalidMethod(format!(
                "method {} has non-contiguous local id {}",
                method.fusen_identity(),
                method.id().get()
            )));
        }
        if !fusen_identities.insert(method.fusen_identity()) {
            return Err(ContractError::InvalidMethod(format!(
                "duplicate Fusen method identity {}",
                method.fusen_identity()
            )));
        }
        if let Some(spring) = method.spring_cloud()
            && !spring_routes.insert((spring.method().as_str(), spring_route_shape(spring.path())))
        {
            return Err(ContractError::InvalidMethod(format!(
                "duplicate Spring Cloud route {} {}",
                spring.method(),
                spring.path()
            )));
        }
    }
    Ok(())
}

fn spring_route_shape(path: &str) -> String {
    let mut shape = String::with_capacity(path.len());
    for segment in path.split('/') {
        shape.push('/');
        if segment.starts_with('{') {
            shape.push_str("{}");
        } else {
            shape.push_str(segment);
        }
    }
    shape.remove(0);
    shape
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProtocolSet, WireProtocol};

    fn spring_get(path: &str, parameters: Vec<SpringCloudParameter>) -> SpringCloudMethod {
        SpringCloudMethod::new(Method::GET, path, parameters).unwrap()
    }

    fn method(id: u16, identity: &str) -> MethodDescriptor {
        MethodDescriptor::new(MethodId::new(id), identity, None).unwrap()
    }

    #[test]
    fn endpoint_normalizes_http_and_https_and_rejects_ambiguous_urls() {
        let plaintext: ServiceEndpoint = "http://EXAMPLE.COM:80/a/../rpc".parse().unwrap();
        assert_eq!(plaintext.as_str(), "http://example.com/rpc");
        let tls: ServiceEndpoint = "https://EXAMPLE.COM:443/a/../rpc".parse().unwrap();
        assert_eq!(tls.as_str(), "https://example.com/rpc");
        for value in [
            "ftp://example.com/rpc",
            "http://user@example.com/rpc",
            "http://example.com:0/rpc",
            "http://example.com/rpc?debug=true",
            "http://example.com/rpc#fragment",
            "/relative",
        ] {
            assert!(value.parse::<ServiceEndpoint>().is_err(), "{value}");
        }
    }

    #[test]
    fn stable_instance_id_has_a_strict_wire_safe_form() {
        let id: InstanceId = "host-1:8080".parse().unwrap();
        assert_eq!(id.as_str(), "host-1:8080");
        for value in ["", "has space", "slash/value", "ü"] {
            assert!(value.parse::<InstanceId>().is_err(), "{value}");
        }
    }

    #[test]
    fn selector_builds_an_unambiguous_stable_identity() {
        let selector =
            ServiceSelector::new("inventory", Some("production".into()), Some("v1".into()))
                .unwrap();
        assert_eq!(selector.identity(), "inventory/production@v1");
        assert!(ServiceSelector::new("bad/value", None, None).is_err());
        let reserved = Metadata::from([("fusen.protocol".into(), "invalid".into())]);
        assert!(selector.with_metadata(reserved).is_err());
    }

    #[test]
    fn spring_mapping_validates_explicit_argument_locations() {
        let mapping = spring_get(
            "/users/{id}",
            vec![
                SpringCloudParameter::new(
                    "id",
                    SpringCloudParameterSource::Path,
                    SpringCloudParameterCardinality::Scalar,
                )
                .unwrap(),
                SpringCloudParameter::new(
                    "filter",
                    SpringCloudParameterSource::Query,
                    SpringCloudParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        assert_eq!(mapping.path(), "/users/{id}");

        let missing_path = SpringCloudMethod::new(
            Method::GET,
            "/users/{id}",
            vec![
                SpringCloudParameter::new(
                    "id",
                    SpringCloudParameterSource::Query,
                    SpringCloudParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        assert!(missing_path.is_err());

        let two_bodies = SpringCloudMethod::new(
            Method::POST,
            "/users",
            vec![
                SpringCloudParameter::new(
                    "first",
                    SpringCloudParameterSource::Body,
                    SpringCloudParameterCardinality::Scalar,
                )
                .unwrap(),
                SpringCloudParameter::new(
                    "second",
                    SpringCloudParameterSource::Body,
                    SpringCloudParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        assert!(two_bodies.is_err());

        let raw_and_fields = SpringCloudMethod::new(
            Method::POST,
            "/users",
            vec![
                SpringCloudParameter::new(
                    "document",
                    SpringCloudParameterSource::Body,
                    SpringCloudParameterCardinality::Scalar,
                )
                .unwrap(),
                SpringCloudParameter::new(
                    "audit",
                    SpringCloudParameterSource::BodyField,
                    SpringCloudParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        assert!(raw_and_fields.is_err());

        let get_body_field = SpringCloudMethod::new(
            Method::GET,
            "/users",
            vec![
                SpringCloudParameter::new(
                    "filter",
                    SpringCloudParameterSource::BodyField,
                    SpringCloudParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        assert!(get_body_field.is_err());

        for path in ["/users/", "/users//active"] {
            assert!(
                SpringCloudMethod::new(Method::GET, path, Vec::new()).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn spring_repeated_cardinality_is_query_only() {
        let repeated = SpringCloudParameter::new(
            "tags",
            SpringCloudParameterSource::Query,
            SpringCloudParameterCardinality::Repeated,
        )
        .unwrap();
        assert_eq!(
            repeated.cardinality(),
            SpringCloudParameterCardinality::Repeated
        );
        assert!(SpringCloudMethod::new(Method::GET, "/items", vec![repeated]).is_ok(),);

        for source in [
            SpringCloudParameterSource::Path,
            SpringCloudParameterSource::BodyField,
            SpringCloudParameterSource::Body,
        ] {
            let parameter = SpringCloudParameter::new(
                "value",
                source,
                SpringCloudParameterCardinality::Repeated,
            )
            .unwrap();
            let path = if source == SpringCloudParameterSource::Path {
                "/items/{value}"
            } else {
                "/items"
            };
            assert!(SpringCloudMethod::new(Method::POST, path, vec![parameter]).is_err());
        }
    }

    #[test]
    fn method_keeps_fusen_identity_independent_from_optional_spring_route() {
        let spring = spring_get("/users", Vec::new());
        let descriptor =
            MethodDescriptor::new(MethodId::new(0), "inventory.users.list", Some(spring)).unwrap();
        assert_eq!(descriptor.fusen_identity(), "inventory.users.list");
        assert!(descriptor.allows_retries());
        assert_eq!(descriptor.spring_cloud().unwrap().path(), "/users");
    }

    #[test]
    fn method_derives_replay_eligibility_from_standard_http_semantics() {
        for (index, method) in [
            Method::GET,
            Method::HEAD,
            Method::OPTIONS,
            Method::PUT,
            Method::DELETE,
        ]
        .into_iter()
        .enumerate()
        {
            let descriptor = MethodDescriptor::new(
                MethodId::new(index as u16),
                method.as_str().to_owned(),
                Some(SpringCloudMethod::new(method, "/users", Vec::new()).unwrap()),
            )
            .unwrap();
            assert!(descriptor.allows_retries());
        }
        for (index, method) in [Method::POST, Method::PATCH].into_iter().enumerate() {
            let descriptor = MethodDescriptor::new(
                MethodId::new(index as u16),
                method.as_str().to_owned(),
                Some(SpringCloudMethod::new(method, "/users", Vec::new()).unwrap()),
            )
            .unwrap();
            assert!(!descriptor.allows_retries());
        }
        assert!(
            !MethodDescriptor::new(MethodId::new(0), "fusen-only", None)
                .unwrap()
                .allows_retries()
        );
    }

    #[test]
    fn service_rejects_duplicate_wire_identities_routes_and_non_contiguous_ids() {
        let selector = ServiceSelector::new("inventory", None, None).unwrap();
        assert!(ServiceDescriptor::new(selector.clone(), vec![method(1, "list")]).is_err());
        assert!(
            ServiceDescriptor::new(selector.clone(), vec![method(0, "list"), method(1, "list")],)
                .is_err()
        );

        let first = MethodDescriptor::new(
            MethodId::new(0),
            "list",
            Some(spring_get("/users", Vec::new())),
        )
        .unwrap();
        let second = MethodDescriptor::new(
            MethodId::new(1),
            "search",
            Some(spring_get("/users", Vec::new())),
        )
        .unwrap();
        assert!(ServiceDescriptor::new(selector, vec![first, second]).is_err());

        let selector = ServiceSelector::new("inventory", None, None).unwrap();
        let by_id = MethodDescriptor::new(
            MethodId::new(0),
            "by-id",
            Some(spring_get(
                "/users/{id}",
                vec![
                    SpringCloudParameter::new(
                        "id",
                        SpringCloudParameterSource::Path,
                        SpringCloudParameterCardinality::Scalar,
                    )
                    .unwrap(),
                ],
            )),
        )
        .unwrap();
        let by_name = MethodDescriptor::new(
            MethodId::new(1),
            "by-name",
            Some(spring_get(
                "/users/{name}",
                vec![
                    SpringCloudParameter::new(
                        "name",
                        SpringCloudParameterSource::Path,
                        SpringCloudParameterCardinality::Scalar,
                    )
                    .unwrap(),
                ],
            )),
        )
        .unwrap();
        assert!(ServiceDescriptor::new(selector, vec![by_id, by_name]).is_err());
    }

    #[test]
    fn registration_and_instance_preserve_stable_instance_identity() {
        let descriptor = Box::leak(Box::new(
            ServiceDescriptor::new(
                ServiceSelector::new("inventory", None, None).unwrap(),
                vec![method(0, "list")],
            )
            .unwrap(),
        ));
        let id = InstanceId::new("inventory-01").unwrap();
        let endpoint: ServiceEndpoint = "http://127.0.0.1:8080".parse().unwrap();
        let registration = ServiceRegistration::new(
            id.clone(),
            descriptor,
            endpoint.clone(),
            ProtocolSet::ALL,
            ServiceWeight::default(),
        )
        .unwrap_err();
        assert!(matches!(
            registration,
            ContractError::UnsupportedServiceProtocol {
                protocol: WireProtocol::SpringCloudV1,
                ..
            }
        ));

        let registration = ServiceRegistration::new(
            id.clone(),
            descriptor,
            endpoint.clone(),
            ProtocolSet::FUSEN_V1,
            ServiceWeight::default(),
        )
        .unwrap();
        assert_eq!(registration.instance_id(), &id);
        assert!(registration.protocols().contains(WireProtocol::FusenV1));

        let instance = ServiceInstance::new(id.clone(), endpoint, ServiceWeight::default());
        assert_eq!(instance.instance_id(), &id);
    }

    #[test]
    fn weight_rejects_non_positive_and_non_finite_values() {
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(ServiceWeight::new(value).is_err());
        }
        assert_eq!(ServiceWeight::new(2.5).unwrap().get(), 2.5);
    }
}
