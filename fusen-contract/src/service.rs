use crate::EndpointCapabilities;
use http::Method;
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

const MAX_IDENTITY_BYTES: usize = 128;

/// Deterministically ordered provider metadata.
pub type Metadata = BTreeMap<String, String>;

/// Invalid HTTP binding or service contract data.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContractError {
    /// A required identity component is not a bounded ASCII token.
    #[error("invalid {0}: expected 1-128 ASCII letters, digits, '.', '_' or '-'")]
    InvalidIdentifier(&'static str),
    /// A custom sensitivity classification is not a bounded ASCII token.
    #[error("invalid sensitivity kind {0:?}: expected 1-64 ASCII letters, digits, '.', '_' or '-'")]
    InvalidSensitivityKind(&'static str),
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
    /// An HTTP binding identifier is not a bounded lowercase segmented token.
    #[error(
        "invalid HTTP binding: expected 1-64 lowercase ASCII letters or digits separated by '-' or '.'"
    )]
    InvalidHttpBinding,
    /// An HTTP version set contained no supported versions.
    #[error("HTTP version set must contain at least one version")]
    EmptyHttpVersionSet,
    /// An HTTP version set contained an unsupported version.
    #[error("unsupported HTTP version {0:?}")]
    UnsupportedHttpVersion(http::Version),
    /// Endpoint capabilities contained no invocation bindings.
    #[error("endpoint capabilities must contain at least one HTTP binding")]
    EmptyHttpBindings,
    /// A service descriptor contains no callable methods.
    #[error("service descriptor must contain at least one method")]
    EmptyMethods,
    /// A method descriptor contains invalid HTTP metadata.
    #[error("invalid method descriptor: {0}")]
    InvalidMethod(String),
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
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServiceSelector {
    service_id: String,
    group: Option<String>,
    version: Option<String>,
    metadata: Metadata,
    identity: String,
}

impl std::fmt::Debug for ServiceSelector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServiceSelector")
            .field("identity", &self.identity)
            .field("metadata_count", &self.metadata.len())
            .finish()
    }
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
/// This value is not a wire identity. [`MethodDescriptor::invocation_name`] remains stable when
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
            Err(ContractError::InvalidEndpoint(
                "expected an absolute HTTP or HTTPS URL without userinfo, a zero port, query, or fragment"
                    .to_owned(),
            ))
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

/// The HTTP location of one ordered invocation argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HttpParameterSource {
    /// A named `{parameter}` path segment.
    Path,
    /// A URL query parameter.
    Query,
    /// A named HTTP header.
    Header,
    /// A named HTTP cookie.
    Cookie,
    /// A named field in a synthesized JSON request body object.
    BodyField,
    /// The complete JSON request body. It cannot be combined with body fields.
    Body,
    /// All otherwise-unmapped URL query parameters as one JSON object.
    /// An operation may contain at most one query map.
    QueryMap,
    /// All otherwise-unmapped HTTP headers as one JSON object.
    /// An operation may contain at most one header map.
    HeaderMap,
}

/// The number of values represented by one HTTP parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HttpParameterCardinality {
    /// At most one scalar value is represented on the wire.
    Scalar,
    /// Zero or more query values are represented as repeated keys.
    Repeated,
}

/// HTTP wire metadata for one ordered invocation argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpParameter {
    name: String,
    source: HttpParameterSource,
    cardinality: HttpParameterCardinality,
}

impl HttpParameter {
    /// Creates validated parameter metadata.
    pub fn new(
        name: impl Into<String>,
        source: HttpParameterSource,
        cardinality: HttpParameterCardinality,
    ) -> Result<Self, ContractError> {
        Ok(Self {
            name: validate_identifier(name.into(), "HTTP parameter name")?,
            source,
            cardinality,
        })
    }

    /// Returns the wire parameter name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the explicit HTTP argument location.
    pub const fn source(&self) -> HttpParameterSource {
        self.source
    }

    /// Returns whether the wire parameter is scalar or repeated.
    pub const fn cardinality(&self) -> HttpParameterCardinality {
        self.cardinality
    }
}

/// Required HTTP mapping for one service method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpOperation {
    method: Method,
    path: String,
    parameters: Vec<HttpParameter>,
    consumes: String,
    produces: String,
}

impl HttpOperation {
    /// Creates and validates an HTTP method, route template, and ordered parameter mapping.
    pub fn new(
        method: Method,
        path: impl Into<String>,
        parameters: Vec<HttpParameter>,
        consumes: impl Into<String>,
        produces: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let path = path.into();
        let consumes = canonical_media_type(consumes.into(), "consumes")?;
        let produces = canonical_media_type(produces.into(), "produces")?;
        validate_http_operation(&method, &path, &parameters)?;
        Ok(Self {
            method,
            path,
            parameters,
            consumes,
            produces,
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

    /// Returns mappings in generated invocation argument order.
    pub fn parameters(&self) -> &[HttpParameter] {
        &self.parameters
    }

    /// Returns the canonical request media type.
    pub fn consumes(&self) -> &str {
        &self.consumes
    }

    /// Returns the canonical response media type.
    pub fn produces(&self) -> &str {
        &self.produces
    }
}

/// Versioned wire metadata and optional process-local policy metadata for one generated service method.
///
/// Equality intentionally excludes sensitivity metadata because it does not participate in the
/// method's wire identity or service contract.
#[derive(Clone)]
pub struct MethodDescriptor {
    id: MethodId,
    invocation_name: String,
    http: HttpOperation,
    sensitivity: Option<crate::MethodSensitivity>,
}

impl MethodDescriptor {
    /// Creates a method descriptor with a required HTTP operation.
    pub fn new(
        id: MethodId,
        invocation_name: impl Into<String>,
        http: HttpOperation,
    ) -> Result<Self, ContractError> {
        let invocation_name =
            validate_identifier(invocation_name.into(), "invocation method name")?;
        Ok(Self {
            id,
            invocation_name,
            http,
            sensitivity: None,
        })
    }

    /// Attaches process-local request and response sensitivity metadata.
    ///
    /// This metadata does not affect wire identity, discovery, or registration.
    pub fn with_sensitivity(mut self, sensitivity: crate::MethodSensitivity) -> Self {
        self.sensitivity = Some(sensitivity);
        self
    }

    /// Returns the process-local declaration-order identifier.
    pub const fn id(&self) -> MethodId {
        self.id
    }

    /// Returns the stable invocation method name.
    pub fn invocation_name(&self) -> &str {
        &self.invocation_name
    }

    /// Returns whether the standard HTTP method permits automatic replay.
    ///
    /// Mappings using POST or PATCH are never replayed.
    pub fn allows_retries(&self) -> bool {
        matches!(
            *self.http.method(),
            Method::GET | Method::HEAD | Method::OPTIONS | Method::PUT | Method::DELETE
        )
    }

    /// Returns the required HTTP operation.
    pub const fn http_operation(&self) -> &HttpOperation {
        &self.http
    }

    /// Returns optional process-local request and response sensitivity metadata.
    pub const fn sensitivity(&self) -> Option<&crate::MethodSensitivity> {
        self.sensitivity.as_ref()
    }
}

impl std::fmt::Debug for MethodDescriptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MethodDescriptor")
            .field("id", &self.id)
            .field("invocation_name", &self.invocation_name)
            .field("http", &self.http)
            .field("has_sensitivity", &self.sensitivity.is_some())
            .finish()
    }
}

impl PartialEq for MethodDescriptor {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.invocation_name == other.invocation_name
            && self.http == other.http
    }
}

impl Eq for MethodDescriptor {}

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
}

/// A complete provider registration submitted to a registry.
#[derive(Clone)]
pub struct ServiceRegistration {
    instance_id: InstanceId,
    descriptor: &'static ServiceDescriptor,
    endpoint: ServiceEndpoint,
    capabilities: EndpointCapabilities,
    weight: ServiceWeight,
    metadata: Metadata,
}

impl std::fmt::Debug for ServiceRegistration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServiceRegistration")
            .field("instance_id", &self.instance_id)
            .field("service", &self.descriptor.identity())
            .field("endpoint", &self.endpoint)
            .field("capabilities", &self.capabilities)
            .field("weight", &self.weight)
            .field("metadata_count", &self.metadata.len())
            .finish()
    }
}

impl ServiceRegistration {
    /// Creates a registration from already validated contract components.
    pub fn new(
        instance_id: InstanceId,
        descriptor: &'static ServiceDescriptor,
        endpoint: ServiceEndpoint,
        capabilities: EndpointCapabilities,
        weight: ServiceWeight,
    ) -> Self {
        Self {
            instance_id,
            descriptor,
            endpoint,
            capabilities,
            weight,
            metadata: Metadata::new(),
        }
    }

    /// Adds provider-owned registration metadata.
    pub fn with_metadata(mut self, metadata: Metadata) -> Result<Self, ContractError> {
        validate_user_metadata(&metadata)?;
        self.metadata = metadata;
        Ok(self)
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

    /// Returns the endpoint's advertised capabilities.
    pub const fn capabilities(&self) -> &EndpointCapabilities {
        &self.capabilities
    }

    /// Returns the service weight.
    pub const fn weight(&self) -> ServiceWeight {
        self.weight
    }

    /// Returns provider-owned registration metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

/// One healthy discovered provider instance.
#[derive(Clone)]
pub struct ServiceInstance {
    instance_id: InstanceId,
    endpoint: ServiceEndpoint,
    capabilities: EndpointCapabilities,
    weight: ServiceWeight,
    metadata: Metadata,
}

impl std::fmt::Debug for ServiceInstance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServiceInstance")
            .field("instance_id", &self.instance_id)
            .field("endpoint", &self.endpoint)
            .field("capabilities", &self.capabilities)
            .field("weight", &self.weight)
            .field("metadata_count", &self.metadata.len())
            .finish()
    }
}

impl ServiceInstance {
    /// Creates a discovered instance from validated identity and endpoint values.
    pub fn new(
        instance_id: InstanceId,
        endpoint: ServiceEndpoint,
        capabilities: EndpointCapabilities,
        weight: ServiceWeight,
    ) -> Self {
        Self {
            instance_id,
            endpoint,
            capabilities,
            weight,
            metadata: Metadata::new(),
        }
    }

    /// Adds user-owned instance metadata.
    pub fn with_metadata(mut self, metadata: Metadata) -> Result<Self, ContractError> {
        validate_user_metadata(&metadata)?;
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

    /// Returns the endpoint's advertised capabilities.
    pub const fn capabilities(&self) -> &EndpointCapabilities {
        &self.capabilities
    }

    /// Returns the selection weight.
    pub const fn weight(&self) -> ServiceWeight {
        self.weight
    }

    /// Returns user-owned metadata, excluding registry capability fields.
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

fn canonical_media_type(value: String, field: &str) -> Result<String, ContractError> {
    value
        .parse::<mime::Mime>()
        .map(|media_type| media_type.to_string())
        .map_err(|_| {
            ContractError::InvalidMethod(format!(
                "invalid {field} media type {value:?}: expected a MIME media type"
            ))
        })
}

fn validate_http_operation(
    method: &Method,
    path: &str,
    parameters: &[HttpParameter],
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
    let placeholders = validate_http_route(path)?;

    let mut names = BTreeSet::new();
    let mut path_parameters = BTreeSet::new();
    let mut body_count = 0;
    let mut body_field_count = 0;
    let mut query_map_count = 0;
    let mut header_map_count = 0;
    for parameter in parameters {
        if !names.insert(parameter.name()) {
            return Err(ContractError::InvalidMethod(format!(
                "duplicate HTTP parameter {}",
                parameter.name()
            )));
        }
        if parameter.cardinality() == HttpParameterCardinality::Repeated
            && parameter.source() != HttpParameterSource::Query
        {
            return Err(ContractError::InvalidMethod(
                "repeated parameters may use only the Query source".into(),
            ));
        }
        match parameter.source() {
            HttpParameterSource::Path => {
                path_parameters.insert(parameter.name().to_owned());
            }
            HttpParameterSource::Body => {
                body_count += 1;
            }
            HttpParameterSource::BodyField => {
                body_field_count += 1;
            }
            HttpParameterSource::QueryMap => {
                query_map_count += 1;
            }
            HttpParameterSource::HeaderMap => {
                header_map_count += 1;
            }
            HttpParameterSource::Query
            | HttpParameterSource::Header
            | HttpParameterSource::Cookie => {}
        }
    }
    if body_count > 1 {
        return Err(ContractError::InvalidMethod(
            "an HTTP operation permits at most one body parameter".into(),
        ));
    }
    if body_count == 1 && body_field_count != 0 {
        return Err(ContractError::InvalidMethod(
            "a complete JSON body cannot be combined with synthesized body fields".into(),
        ));
    }
    if query_map_count > 1 {
        return Err(ContractError::InvalidMethod(
            "an HTTP operation permits at most one QueryMap parameter".into(),
        ));
    }
    if header_map_count > 1 {
        return Err(ContractError::InvalidMethod(
            "an HTTP operation permits at most one HeaderMap parameter".into(),
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
            "HTTP route placeholders do not match Path parameters".into(),
        ));
    }
    Ok(())
}

fn validate_http_route(path: &str) -> Result<BTreeSet<String>, ContractError> {
    let invalid = |reason: &str| {
        ContractError::InvalidMethod(format!("invalid HTTP route {path:?}: {reason}"))
    };
    if !path.starts_with('/') {
        return Err(invalid("routes must be absolute"));
    }
    if path.contains(['?', '#']) {
        return Err(invalid("query strings and fragments are not allowed"));
    }
    if path.contains("//") {
        return Err(invalid("empty path segments are not canonical"));
    }
    if path.len() > 1 && path.ends_with('/') {
        return Err(invalid("a trailing slash is not canonical"));
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
            if !is_identity_component(name) || !placeholders.insert(name.to_owned()) {
                return Err(invalid(
                    "path parameters must be unique full ASCII token segments",
                ));
            }
        } else {
            if segment.contains(['{', '}']) {
                return Err(invalid("path parameters must occupy complete segments"));
            }
            decode_route_literal(segment).map_err(invalid)?;
        }
    }
    Ok(placeholders)
}

fn decode_route_literal(segment: &str) -> Result<String, &'static str> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            let Some((&high, rest)) = bytes.get(index + 1).zip(bytes.get(index + 2..)) else {
                return Err("percent escapes must contain two uppercase hexadecimal digits");
            };
            let Some(&low) = rest.first() else {
                return Err("percent escapes must contain two uppercase hexadecimal digits");
            };
            let Some(high) = uppercase_hex_value(high) else {
                return Err("percent escapes must contain two uppercase hexadecimal digits");
            };
            let Some(low) = uppercase_hex_value(low) else {
                return Err("percent escapes must contain two uppercase hexadecimal digits");
            };
            let value = high * 16 + low;
            if value.is_ascii() {
                return Err("ASCII route characters must not be percent encoded");
            }
            decoded.push(value);
            index += 3;
        } else {
            if !is_route_pchar(byte) {
                return Err("literal segments must use ASCII RFC3986 path characters");
            }
            decoded.push(byte);
            index += 1;
        }
    }
    let decoded =
        String::from_utf8(decoded).map_err(|_| "percent-encoded route text must be valid UTF-8")?;
    if decoded == "." || decoded == ".." {
        return Err("dot path segments are not allowed");
    }
    if decoded.chars().any(char::is_whitespace) || decoded.chars().any(char::is_control) {
        return Err("route literals must not contain whitespace or control characters");
    }
    Ok(decoded)
}

fn uppercase_hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_route_pchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
        )
}

fn validate_methods(methods: &[MethodDescriptor]) -> Result<(), ContractError> {
    if methods.is_empty() {
        return Err(ContractError::EmptyMethods);
    }
    let mut invocation_names = BTreeSet::new();
    let mut http_routes = BTreeSet::new();
    for (index, method) in methods.iter().enumerate() {
        if method.id().index() != index {
            return Err(ContractError::InvalidMethod(format!(
                "method {} has non-contiguous local id {}",
                method.invocation_name(),
                method.id().get()
            )));
        }
        if !invocation_names.insert(method.invocation_name()) {
            return Err(ContractError::InvalidMethod(format!(
                "duplicate invocation method name {}",
                method.invocation_name()
            )));
        }
        let operation = method.http_operation();
        if !http_routes.insert((
            operation.method().as_str(),
            http_route_shape(operation.path()),
        )) {
            return Err(ContractError::InvalidMethod(format!(
                "duplicate HTTP route {} {}",
                operation.method(),
                operation.path()
            )));
        }
    }
    Ok(())
}

fn http_route_shape(path: &str) -> String {
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
    use crate::{
        EndpointCapabilities, HttpBindingId, HttpVersionSet, MethodSensitivity, SensitiveArgument,
        SensitiveShape, SensitivityKind,
    };

    fn token_shape() -> SensitiveShape {
        SensitiveShape::Kind(SensitivityKind::TOKEN)
    }

    fn public_shape() -> SensitiveShape {
        SensitiveShape::Kind(SensitivityKind::PUBLIC)
    }

    fn http_operation(
        method: Method,
        path: &str,
        parameters: Vec<HttpParameter>,
    ) -> Result<HttpOperation, ContractError> {
        HttpOperation::new(
            method,
            path,
            parameters,
            "application/json",
            "application/json",
        )
    }

    fn http_get(path: &str, parameters: Vec<HttpParameter>) -> HttpOperation {
        http_operation(Method::GET, path, parameters).unwrap()
    }

    fn method(id: u16, identity: &str) -> MethodDescriptor {
        MethodDescriptor::new(
            MethodId::new(id),
            identity,
            http_get(&format!("/method-{id}"), Vec::new()),
        )
        .unwrap()
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

        let sensitive = "http://user:secret@example.com/rpc"
            .parse::<ServiceEndpoint>()
            .unwrap_err();
        assert!(!format!("{sensitive}").contains("secret"));
        assert!(!format!("{sensitive:?}").contains("secret"));
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
        let reserved = Metadata::from([("fusen.http.binding".into(), "invalid".into())]);
        assert!(selector.with_metadata(reserved).is_err());
    }

    #[test]
    fn http_operation_validates_explicit_argument_locations() {
        let mapping = http_get(
            "/users/{id}",
            vec![
                HttpParameter::new(
                    "id",
                    HttpParameterSource::Path,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
                HttpParameter::new(
                    "filter",
                    HttpParameterSource::Query,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        assert_eq!(mapping.path(), "/users/{id}");

        let missing_path = http_operation(
            Method::GET,
            "/users/{id}",
            vec![
                HttpParameter::new(
                    "id",
                    HttpParameterSource::Query,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        assert!(missing_path.is_err());

        let two_bodies = http_operation(
            Method::POST,
            "/users",
            vec![
                HttpParameter::new(
                    "first",
                    HttpParameterSource::Body,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
                HttpParameter::new(
                    "second",
                    HttpParameterSource::Body,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        assert!(two_bodies.is_err());

        let raw_and_fields = http_operation(
            Method::POST,
            "/users",
            vec![
                HttpParameter::new(
                    "document",
                    HttpParameterSource::Body,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
                HttpParameter::new(
                    "audit",
                    HttpParameterSource::BodyField,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        assert!(raw_and_fields.is_err());

        let get_body_field = http_operation(
            Method::GET,
            "/users",
            vec![
                HttpParameter::new(
                    "filter",
                    HttpParameterSource::BodyField,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        assert!(get_body_field.is_err());

        for path in ["/users/", "/users//active"] {
            assert!(
                http_operation(Method::GET, path, Vec::new()).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn http_operation_canonicalizes_media_types_and_supports_http_sources() {
        let operation = HttpOperation::new(
            Method::POST,
            "/items",
            vec![
                HttpParameter::new(
                    "tenant",
                    HttpParameterSource::Header,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
                HttpParameter::new(
                    "session",
                    HttpParameterSource::Cookie,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
                HttpParameter::new(
                    "query",
                    HttpParameterSource::QueryMap,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
                HttpParameter::new(
                    "headers",
                    HttpParameterSource::HeaderMap,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
            "application/json; charset=utf-8",
            "application/vnd.fusen+json",
        )
        .unwrap();
        assert_eq!(operation.consumes(), "application/json; charset=utf-8");
        assert_eq!(operation.produces(), "application/vnd.fusen+json");
        assert!(
            HttpOperation::new(
                Method::GET,
                "/items",
                Vec::new(),
                "not a media type",
                "application/json",
            )
            .is_err()
        );
    }

    #[test]
    fn repeated_cardinality_is_query_only() {
        let repeated = HttpParameter::new(
            "tags",
            HttpParameterSource::Query,
            HttpParameterCardinality::Repeated,
        )
        .unwrap();
        assert_eq!(repeated.cardinality(), HttpParameterCardinality::Repeated);
        assert!(http_operation(Method::GET, "/items", vec![repeated]).is_ok(),);

        for source in [
            HttpParameterSource::Path,
            HttpParameterSource::Header,
            HttpParameterSource::Cookie,
            HttpParameterSource::BodyField,
            HttpParameterSource::Body,
            HttpParameterSource::QueryMap,
            HttpParameterSource::HeaderMap,
        ] {
            let parameter =
                HttpParameter::new("value", source, HttpParameterCardinality::Repeated).unwrap();
            let path = if source == HttpParameterSource::Path {
                "/items/{value}"
            } else {
                "/items"
            };
            assert!(http_operation(Method::POST, path, vec![parameter]).is_err());
        }
    }

    #[test]
    fn http_operation_allows_at_most_one_parameter_map_per_source() {
        for source in [
            HttpParameterSource::QueryMap,
            HttpParameterSource::HeaderMap,
        ] {
            let parameters = ["first", "second"]
                .into_iter()
                .map(|name| {
                    HttpParameter::new(name, source, HttpParameterCardinality::Scalar).unwrap()
                })
                .collect();
            let error = http_operation(Method::GET, "/items", parameters).unwrap_err();
            assert!(error.to_string().contains("at most one"));
        }
    }

    #[test]
    fn http_routes_require_canonical_rfc3986_literals() {
        for path in [
            "/",
            "/users/a-._~!$&'()*+,;=:@",
            "/%E7%94%A8",
            "/users/{id}",
        ] {
            let parameters = if path.contains("{id}") {
                vec![
                    HttpParameter::new(
                        "id",
                        HttpParameterSource::Path,
                        HttpParameterCardinality::Scalar,
                    )
                    .unwrap(),
                ]
            } else {
                Vec::new()
            };
            assert!(
                http_operation(Method::GET, path, parameters).is_ok(),
                "{path}"
            );
        }

        for path in [
            "relative",
            "/raw-用户",
            "/has space",
            "/has\\backslash",
            "/control\u{7f}",
            "/%ZZ",
            "/%e7%94%a8",
            "/%41",
            "/%C3",
            "/%C2%A0",
            "/%C2%85",
            "/.",
            "/..",
        ] {
            assert!(
                http_operation(Method::GET, path, Vec::new()).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn method_keeps_invocation_name_independent_from_http_route() {
        let operation = http_get("/users", Vec::new());
        let descriptor =
            MethodDescriptor::new(MethodId::new(0), "inventory.users.list", operation).unwrap();
        assert_eq!(descriptor.invocation_name(), "inventory.users.list");
        assert!(descriptor.allows_retries());
        assert_eq!(descriptor.http_operation().path(), "/users");
    }

    #[test]
    fn method_sensitivity_is_optional_process_local_metadata() {
        let operation = http_get(
            "/users/{id}",
            vec![
                HttpParameter::new(
                    "id",
                    HttpParameterSource::Path,
                    HttpParameterCardinality::Scalar,
                )
                .unwrap(),
            ],
        );
        let plain =
            MethodDescriptor::new(MethodId::new(0), "inventory.users.get", operation.clone())
                .unwrap();
        assert!(plain.sensitivity().is_none());

        let classified = MethodDescriptor::new(MethodId::new(0), "inventory.users.get", operation)
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(
                vec![SensitiveArgument::new("id", token_shape)],
                Some(public_shape),
            ));

        let sensitivity = classified.sensitivity().unwrap();
        assert_eq!(sensitivity.arguments()[0].name(), "id");
        assert!(matches!(
            sensitivity.arguments()[0].shape(),
            SensitiveShape::Kind(SensitivityKind::TOKEN)
        ));
        assert!(matches!(
            sensitivity.response_shape(),
            Some(SensitiveShape::Kind(SensitivityKind::PUBLIC))
        ));
        assert_eq!(classified.id(), plain.id());
        assert_eq!(classified.invocation_name(), plain.invocation_name());
        assert_eq!(classified.http_operation(), plain.http_operation());
        assert_eq!(classified.allows_retries(), plain.allows_retries());
        assert_eq!(classified, plain);
        assert!(format!("{classified:?}").contains("has_sensitivity: true"));

        let selector = ServiceSelector::new("inventory", None, None).unwrap();
        let plain_service = ServiceDescriptor::new(selector.clone(), vec![plain]).unwrap();
        let classified_service = ServiceDescriptor::new(selector, vec![classified]).unwrap();
        assert_eq!(plain_service.identity(), classified_service.identity());
        assert_eq!(plain_service.methods(), classified_service.methods());
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
                http_operation(method, "/users", Vec::new()).unwrap(),
            )
            .unwrap();
            assert!(descriptor.allows_retries());
        }
        for (index, method) in [Method::POST, Method::PATCH].into_iter().enumerate() {
            let descriptor = MethodDescriptor::new(
                MethodId::new(index as u16),
                method.as_str().to_owned(),
                http_operation(method, "/users", Vec::new()).unwrap(),
            )
            .unwrap();
            assert!(!descriptor.allows_retries());
        }
    }

    #[test]
    fn service_rejects_duplicate_wire_identities_routes_and_non_contiguous_ids() {
        let selector = ServiceSelector::new("inventory", None, None).unwrap();
        assert!(ServiceDescriptor::new(selector.clone(), vec![method(1, "list")]).is_err());
        assert!(
            ServiceDescriptor::new(selector.clone(), vec![method(0, "list"), method(1, "list")],)
                .is_err()
        );

        let first = MethodDescriptor::new(MethodId::new(0), "list", http_get("/users", Vec::new()))
            .unwrap();
        let second =
            MethodDescriptor::new(MethodId::new(1), "search", http_get("/users", Vec::new()))
                .unwrap();
        assert!(ServiceDescriptor::new(selector, vec![first, second]).is_err());

        let selector = ServiceSelector::new("inventory", None, None).unwrap();
        let by_id = MethodDescriptor::new(
            MethodId::new(0),
            "by-id",
            http_get(
                "/users/{id}",
                vec![
                    HttpParameter::new(
                        "id",
                        HttpParameterSource::Path,
                        HttpParameterCardinality::Scalar,
                    )
                    .unwrap(),
                ],
            ),
        )
        .unwrap();
        let by_name = MethodDescriptor::new(
            MethodId::new(1),
            "by-name",
            http_get(
                "/users/{name}",
                vec![
                    HttpParameter::new(
                        "name",
                        HttpParameterSource::Path,
                        HttpParameterCardinality::Scalar,
                    )
                    .unwrap(),
                ],
            ),
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
        let unsupported = EndpointCapabilities::new(
            HttpVersionSet::HTTP_1_1,
            [HttpBindingId::new("vendor-json-v1").unwrap()],
            false,
        )
        .unwrap();
        let custom_registration = ServiceRegistration::new(
            id.clone(),
            descriptor,
            endpoint.clone(),
            unsupported,
            ServiceWeight::default(),
        );
        assert_eq!(custom_registration.capabilities().bindings().len(), 1);

        let registration = ServiceRegistration::new(
            id.clone(),
            descriptor,
            endpoint.clone(),
            EndpointCapabilities::default(),
            ServiceWeight::default(),
        )
        .with_metadata(Metadata::from([("zone".into(), "east".into())]))
        .unwrap();
        assert_eq!(registration.instance_id(), &id);
        assert_eq!(registration.metadata()["zone"], "east");
        assert!(
            registration
                .capabilities()
                .supports_binding(&HttpBindingId::default())
        );

        let instance = ServiceInstance::new(
            id.clone(),
            endpoint,
            EndpointCapabilities::default(),
            ServiceWeight::default(),
        );
        assert_eq!(instance.instance_id(), &id);
        let reserved = Metadata::from([("fusen.http.bindings".into(), "http-json-v1".into())]);
        assert!(instance.with_metadata(reserved).is_err());
    }

    #[test]
    fn service_carrier_debug_never_expands_metadata_values() {
        let selector = ServiceSelector::new("inventory", None, None)
            .unwrap()
            .with_metadata(Metadata::from([(
                "credential".into(),
                "private-selector-token".into(),
            )]))
            .unwrap();
        let descriptor = Box::leak(Box::new(
            ServiceDescriptor::new(selector.clone(), vec![method(0, "list")]).unwrap(),
        ));
        let endpoint: ServiceEndpoint = "http://127.0.0.1:8080".parse().unwrap();
        let registration = ServiceRegistration::new(
            InstanceId::new("inventory-01").unwrap(),
            descriptor,
            endpoint.clone(),
            EndpointCapabilities::default(),
            ServiceWeight::default(),
        )
        .with_metadata(Metadata::from([(
            "credential".into(),
            "private-registration-token".into(),
        )]))
        .unwrap();
        let instance = ServiceInstance::new(
            InstanceId::new("inventory-02").unwrap(),
            endpoint,
            EndpointCapabilities::default(),
            ServiceWeight::default(),
        )
        .with_metadata(Metadata::from([(
            "credential".into(),
            "private-instance-token".into(),
        )]))
        .unwrap();

        for debug in [
            format!("{selector:?}"),
            format!("{registration:?}"),
            format!("{instance:?}"),
        ] {
            assert!(debug.contains("metadata_count: 1"));
            assert!(!debug.contains("private-"));
        }
    }

    #[test]
    fn weight_rejects_non_positive_and_non_finite_values() {
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(ServiceWeight::new(value).is_err());
        }
        assert_eq!(ServiceWeight::new(2.5).unwrap().get(), 2.5);
    }
}
