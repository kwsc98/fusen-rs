use crate::{NacosConfig, client_props, validate_application_name};
use fusen_contract::{
    EndpointCapabilities, HttpBindingId, HttpVersionSet, InstanceId, ServiceEndpoint,
    ServiceInstance, ServiceRegistration, ServiceSelector, ServiceWeight,
};
use fusen_register::{
    RegistrationHandle, RegistrationRequest, Registry, RegistryFuture, SubscriptionHandle,
    SubscriptionRequest,
    directory::{DirectoryPublisher, directory},
    error::{RegistryError, RegistryErrorKind, RegistryOperation},
    provider,
};
use nacos_sdk::api::error::Error as NacosError;
use nacos_sdk::api::naming::{
    NamingChangeEvent, NamingEventListener, NamingService, NamingServiceBuilder,
    ServiceInstance as NacosServiceInstance,
};
use std::sync::{Arc, Mutex};

const META_SCHEME: &str = "fusen.scheme";
const META_BASE_PATH: &str = "fusen.base_path";
const META_SERVICE_ID: &str = "fusen.service_id";
const META_VERSION: &str = "fusen.version";
const META_GROUP: &str = "fusen.group";
const META_INSTANCE_ID: &str = "fusen.instance_id";
const META_HTTP_BINDINGS: &str = "fusen.http.bindings";
const META_HTTP_VERSIONS: &str = "fusen.http.versions";
const META_INVOCATION_CONTROLS: &str = "fusen.invocation-controls";
const INVOCATION_CONTROLS_V1: &str = "v1";
const DEFAULT_GROUP: &str = "DEFAULT_GROUP";

/// Provider-specific conventions used when interoperating with Nacos services.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NacosConvention {
    /// Requires Fusen endpoint capability metadata.
    #[default]
    Canonical,
    /// Accepts Spring Cloud instances that omit Fusen endpoint capability metadata.
    SpringCloud,
}

/// Nacos-backed service registry and discovery provider.
#[derive(Clone)]
pub struct NacosRegistry {
    naming: Arc<dyn NamingOperations>,
    convention: NacosConvention,
}

impl NacosRegistry {
    /// Connects a Nacos naming client for one application.
    ///
    /// Configuration is validated before the SDK performs network I/O.
    pub async fn connect(
        application_name: impl Into<String>,
        config: NacosConfig,
    ) -> Result<Self, RegistryError> {
        let application_name = application_name.into();
        config.validate().map_err(|error| {
            RegistryError::new(
                RegistryOperation::PrepareRegistration,
                RegistryErrorKind::InvalidResource,
                error,
            )
        })?;
        validate_application_name(&application_name).map_err(|message| {
            RegistryError::message(
                RegistryOperation::PrepareRegistration,
                RegistryErrorKind::InvalidResource,
                message,
            )
        })?;
        let builder = NamingServiceBuilder::new(client_props(&config, &application_name));
        let builder = if config.username().is_some() {
            builder.enable_auth_plugin_http()
        } else {
            builder
        };
        let service = builder
            .build()
            .await
            .map_err(|error| provider_error(RegistryOperation::PrepareRegistration, error))?;
        Ok(Self {
            naming: Arc::new(SdkNamingOperations {
                service: Arc::new(service),
            }),
            convention: NacosConvention::Canonical,
        })
    }

    /// Selects the conventions used to encode and decode Nacos instances.
    pub fn with_convention(mut self, convention: NacosConvention) -> Self {
        self.convention = convention;
        self
    }

    /// Returns the configured Nacos interoperability convention.
    pub const fn convention(&self) -> NacosConvention {
        self.convention
    }
}

impl std::fmt::Debug for NacosRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NacosRegistry")
            .field("convention", &self.convention)
            .finish_non_exhaustive()
    }
}

impl Registry for NacosRegistry {
    fn prepare_registration(
        &self,
        request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError> {
        let registration = request.into_registration();
        let service_name = service_name(registration.selector());
        let group = service_group(registration.selector());
        let instance = build_instance(&registration)?;
        let activate_naming = self.naming.clone();
        let activate_service_name = service_name.clone();
        let activate_group = group.clone();
        let activate_instance = instance.clone();
        let close_naming = self.naming.clone();

        Ok(provider::registration(
            async move {
                activate_naming
                    .register(activate_service_name, activate_group, activate_instance)
                    .await
            },
            move || async move { close_naming.deregister(service_name, group, instance).await },
        ))
    }

    fn prepare_subscription(
        &self,
        request: SubscriptionRequest,
    ) -> Result<SubscriptionHandle, RegistryError> {
        let selector = request.into_selector();
        let service_name = service_name(&selector);
        let group = service_group(&selector);
        let clusters = Vec::new();
        let (publisher, directory) = directory();
        let snapshot_gate = Arc::new(SnapshotGate::new(publisher));
        let listener: Arc<dyn NamingEventListener> = Arc::new(ServiceChangeListener {
            snapshot_gate: snapshot_gate.clone(),
            selector: selector.clone(),
            convention: self.convention,
        });

        let activate_naming = self.naming.clone();
        let activate_service_name = service_name.clone();
        let activate_group = group.clone();
        let activate_clusters = clusters.clone();
        let activate_listener = listener.clone();
        let close_naming = self.naming.clone();
        let convention = self.convention;

        Ok(provider::subscription(
            directory,
            async move {
                activate_naming
                    .subscribe(
                        activate_service_name.clone(),
                        activate_group.clone(),
                        activate_clusters.clone(),
                        activate_listener,
                    )
                    .await?;
                let instances = activate_naming
                    .select(activate_service_name, activate_group, activate_clusters)
                    .await?;
                snapshot_gate.initialize(to_service_instances(instances, &selector, convention))
            },
            move || async move {
                close_naming
                    .unsubscribe(service_name, group, clusters, listener)
                    .await
            },
        ))
    }
}

trait NamingOperations: Send + Sync {
    fn register(
        &self,
        service_name: String,
        group: Option<String>,
        instance: NacosServiceInstance,
    ) -> RegistryFuture<()>;

    fn deregister(
        &self,
        service_name: String,
        group: Option<String>,
        instance: NacosServiceInstance,
    ) -> RegistryFuture<()>;

    fn subscribe(
        &self,
        service_name: String,
        group: Option<String>,
        clusters: Vec<String>,
        listener: Arc<dyn NamingEventListener>,
    ) -> RegistryFuture<()>;

    fn unsubscribe(
        &self,
        service_name: String,
        group: Option<String>,
        clusters: Vec<String>,
        listener: Arc<dyn NamingEventListener>,
    ) -> RegistryFuture<()>;

    fn select(
        &self,
        service_name: String,
        group: Option<String>,
        clusters: Vec<String>,
    ) -> RegistryFuture<Vec<NacosServiceInstance>>;
}

struct SdkNamingOperations {
    service: Arc<NamingService>,
}

impl NamingOperations for SdkNamingOperations {
    fn register(
        &self,
        service_name: String,
        group: Option<String>,
        instance: NacosServiceInstance,
    ) -> RegistryFuture<()> {
        let service = self.service.clone();
        Box::pin(async move {
            service
                .register_instance(service_name, group, instance)
                .await
                .map_err(|error| provider_error(RegistryOperation::ActivateRegistration, error))
        })
    }

    fn deregister(
        &self,
        service_name: String,
        group: Option<String>,
        instance: NacosServiceInstance,
    ) -> RegistryFuture<()> {
        let service = self.service.clone();
        Box::pin(async move {
            service
                .deregister_instance(service_name, group, instance)
                .await
                .map_err(|error| provider_error(RegistryOperation::CloseRegistration, error))
        })
    }

    fn subscribe(
        &self,
        service_name: String,
        group: Option<String>,
        clusters: Vec<String>,
        listener: Arc<dyn NamingEventListener>,
    ) -> RegistryFuture<()> {
        let service = self.service.clone();
        Box::pin(async move {
            service
                .subscribe(service_name, group, clusters, listener)
                .await
                .map_err(|error| provider_error(RegistryOperation::ActivateSubscription, error))
        })
    }

    fn unsubscribe(
        &self,
        service_name: String,
        group: Option<String>,
        clusters: Vec<String>,
        listener: Arc<dyn NamingEventListener>,
    ) -> RegistryFuture<()> {
        let service = self.service.clone();
        Box::pin(async move {
            service
                .unsubscribe(service_name, group, clusters, listener)
                .await
                .map_err(|error| provider_error(RegistryOperation::CloseSubscription, error))
        })
    }

    fn select(
        &self,
        service_name: String,
        group: Option<String>,
        clusters: Vec<String>,
    ) -> RegistryFuture<Vec<NacosServiceInstance>> {
        let service = self.service.clone();
        Box::pin(async move {
            service
                .select_instances(service_name, group, clusters, false, true)
                .await
                .map_err(|error| provider_error(RegistryOperation::ActivateSubscription, error))
        })
    }
}

fn provider_error(operation: RegistryOperation, error: NacosError) -> RegistryError {
    let kind = match &error {
        NacosError::InvalidParam(_, _) | NacosError::WrongServerAddress(_) => {
            RegistryErrorKind::InvalidResource
        }
        NacosError::Serialization(_) => RegistryErrorKind::Internal,
        _ => RegistryErrorKind::Unavailable,
    };
    RegistryError::new(operation, kind, error)
}

#[derive(Default)]
struct SnapshotGateState {
    initialized: bool,
    pending: Option<Vec<ServiceInstance>>,
}

struct SnapshotGate {
    publisher: DirectoryPublisher,
    state: Mutex<SnapshotGateState>,
}

impl SnapshotGate {
    fn new(publisher: DirectoryPublisher) -> Self {
        Self {
            publisher,
            state: Mutex::new(SnapshotGateState::default()),
        }
    }

    fn initialize(&self, initial: Vec<ServiceInstance>) -> Result<(), RegistryError> {
        let mut state = self.state.lock().map_err(|_| {
            RegistryError::message(
                RegistryOperation::Directory,
                RegistryErrorKind::Internal,
                "Nacos snapshot gate lock was poisoned",
            )
        })?;
        let snapshot = state.pending.take().unwrap_or(initial);
        self.publisher.publish_ready(snapshot)?;
        state.initialized = true;
        Ok(())
    }

    fn update(&self, instances: Vec<ServiceInstance>) -> Result<(), RegistryError> {
        let mut state = self.state.lock().map_err(|_| {
            RegistryError::message(
                RegistryOperation::Directory,
                RegistryErrorKind::Internal,
                "Nacos snapshot gate lock was poisoned",
            )
        })?;
        if state.initialized {
            self.publisher.publish_ready(instances)?;
        } else {
            state.pending = Some(instances);
        }
        Ok(())
    }
}

struct ServiceChangeListener {
    snapshot_gate: Arc<SnapshotGate>,
    selector: ServiceSelector,
    convention: NacosConvention,
}

impl NamingEventListener for ServiceChangeListener {
    fn event(&self, event: Arc<NamingChangeEvent>) {
        let instances = event
            .instances
            .clone()
            .map(|instances| to_service_instances(instances, &self.selector, self.convention))
            .unwrap_or_default();
        if let Err(error) = self.snapshot_gate.update(instances) {
            tracing::warn!(
                error = %error,
                service = %event.service_name,
                "Nacos directory update rejected"
            );
        }
    }
}

fn to_service_instances(
    instances: Vec<NacosServiceInstance>,
    selector: &ServiceSelector,
    convention: NacosConvention,
) -> Vec<ServiceInstance> {
    instances
        .into_iter()
        .filter(|instance| instance.healthy && instance.enabled && instance.weight > 0.0)
        .filter(|instance| matches_selector(instance, selector, convention))
        .filter_map(|instance| to_service_instance(instance, convention))
        .collect()
}

fn matches_selector(
    instance: &NacosServiceInstance,
    selector: &ServiceSelector,
    convention: NacosConvention,
) -> bool {
    if instance.metadata.get(META_VERSION).map(String::as_str) != selector.version() {
        return false;
    }
    match instance.metadata.get(META_SERVICE_ID) {
        Some(service_id) => service_id == selector.service_id(),
        None => convention == NacosConvention::SpringCloud,
    }
}

fn to_service_instance(
    instance: NacosServiceInstance,
    convention: NacosConvention,
) -> Option<ServiceInstance> {
    let port = u16::try_from(instance.port)
        .ok()
        .filter(|port| *port != 0)?;
    let scheme = instance
        .metadata
        .get(META_SCHEME)
        .map(String::as_str)
        .unwrap_or("http");
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let host = if instance.ip.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{}]", instance.ip)
    } else {
        instance.ip.clone()
    };
    let mut url = url::Url::parse(&format!("{scheme}://{host}:{port}")).ok()?;
    if let Some(path) = instance.metadata.get(META_BASE_PATH) {
        let mut segments = url.path_segments_mut().ok()?;
        segments.clear();
        for segment in path.trim_matches('/').split('/') {
            if segment.is_empty() {
                continue;
            }
            let segment = percent_encoding::percent_decode_str(segment)
                .decode_utf8()
                .ok()?;
            segments.push(&segment);
        }
    }
    let instance_id = instance
        .metadata
        .get(META_INSTANCE_ID)
        .cloned()
        .or(instance.instance_id.clone())
        .and_then(|value| InstanceId::new(value).ok())
        .or_else(|| InstanceId::new(format!("nacos:{}:{port}", instance.ip)).ok())?;
    let endpoint = ServiceEndpoint::try_from(url).ok()?;
    let capabilities = decode_capabilities(&instance.metadata, convention)?;
    let weight = ServiceWeight::new(instance.weight).ok()?;
    let user_metadata = instance
        .metadata
        .into_iter()
        .filter(|(key, _)| !key.starts_with("fusen."))
        .collect();
    ServiceInstance::new(instance_id, endpoint, capabilities, weight)
        .with_metadata(user_metadata)
        .ok()
}

fn decode_capabilities(
    metadata: &std::collections::HashMap<String, String>,
    convention: NacosConvention,
) -> Option<EndpointCapabilities> {
    let bindings = metadata.get(META_HTTP_BINDINGS);
    let versions = metadata.get(META_HTTP_VERSIONS);
    let controls = metadata.get(META_INVOCATION_CONTROLS);
    if bindings.is_none() && versions.is_none() && controls.is_none() {
        return (convention == NacosConvention::SpringCloud).then(EndpointCapabilities::default);
    }
    let bindings = decode_bindings(bindings?)?;
    let versions = decode_http_versions(versions?)?;
    let invocation_controls = match controls.map(String::as_str) {
        None => false,
        Some(INVOCATION_CONTROLS_V1) => true,
        Some(_) => return None,
    };
    EndpointCapabilities::new(versions, bindings, invocation_controls).ok()
}

fn decode_bindings(value: &str) -> Option<Vec<HttpBindingId>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut bindings = Vec::new();
    for value in value.split(',') {
        let binding = HttpBindingId::new(value).ok()?;
        if !seen.insert(binding.clone()) {
            return None;
        }
        bindings.push(binding);
    }
    (!bindings.is_empty()).then_some(bindings)
}

fn decode_http_versions(value: &str) -> Option<HttpVersionSet> {
    let mut http1 = false;
    let mut http2 = false;
    for version in value.split(',') {
        let seen = match version {
            "1.1" => &mut http1,
            "2" => &mut http2,
            _ => return None,
        };
        if *seen {
            return None;
        }
        *seen = true;
    }
    match (http1, http2) {
        (true, true) => Some(HttpVersionSet::ALL),
        (true, false) => Some(HttpVersionSet::HTTP_1_1),
        (false, true) => Some(HttpVersionSet::HTTP_2),
        (false, false) => None,
    }
}

fn service_name(selector: &ServiceSelector) -> String {
    selector.service_id().to_owned()
}

fn service_group(selector: &ServiceSelector) -> Option<String> {
    Some(selector.group().unwrap_or(DEFAULT_GROUP).to_owned())
}

fn build_instance(
    registration: &ServiceRegistration,
) -> Result<NacosServiceInstance, RegistryError> {
    let url = registration.endpoint().as_url();
    let ip = url.host_str().ok_or_else(|| {
        RegistryError::message(
            RegistryOperation::PrepareRegistration,
            RegistryErrorKind::InvalidResource,
            "advertised endpoint has no host",
        )
    })?;
    let ip = ip
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(ip);
    let port = url.port_or_known_default().ok_or_else(|| {
        RegistryError::message(
            RegistryOperation::PrepareRegistration,
            RegistryErrorKind::InvalidResource,
            "advertised endpoint has no port",
        )
    })?;
    let mut metadata = registration
        .selector()
        .metadata()
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    metadata.extend(
        registration
            .metadata()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    metadata.insert(META_SCHEME.into(), url.scheme().into());
    metadata.insert(
        META_SERVICE_ID.into(),
        registration.selector().service_id().to_owned(),
    );
    metadata.insert(
        META_INSTANCE_ID.into(),
        registration.instance_id().to_string(),
    );
    metadata.insert(
        META_HTTP_BINDINGS.into(),
        registration
            .capabilities()
            .bindings()
            .iter()
            .map(HttpBindingId::as_str)
            .collect::<Vec<_>>()
            .join(","),
    );
    metadata.insert(
        META_HTTP_VERSIONS.into(),
        encode_http_versions(registration.capabilities().http_versions()).into(),
    );
    if registration.capabilities().invocation_controls() {
        metadata.insert(
            META_INVOCATION_CONTROLS.into(),
            INVOCATION_CONTROLS_V1.into(),
        );
    }
    if url.path() != "/" && !url.path().is_empty() {
        metadata.insert(META_BASE_PATH.into(), url.path().into());
    }
    if let Some(version) = registration.selector().version() {
        metadata.insert(META_VERSION.into(), version.to_owned());
    }
    if let Some(group) = registration.selector().group() {
        metadata.insert(META_GROUP.into(), group.to_owned());
    }
    Ok(NacosServiceInstance {
        instance_id: Some(registration.instance_id().to_string()),
        ip: ip.to_owned(),
        port: i32::from(port),
        weight: registration.weight().get(),
        metadata,
        ..NacosServiceInstance::default()
    })
}

fn encode_http_versions(versions: HttpVersionSet) -> &'static str {
    if versions == HttpVersionSet::HTTP_1_1 {
        "1.1"
    } else if versions == HttpVersionSet::HTTP_2 {
        "2"
    } else {
        "1.1,2"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusen_contract::{HttpOperation, MethodDescriptor, MethodId, ServiceDescriptor};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::{Notify, oneshot};

    fn selector() -> ServiceSelector {
        ServiceSelector::new("demo", Some("prod".into()), Some("1".into())).unwrap()
    }

    fn registration_at(endpoint: &str) -> Arc<ServiceRegistration> {
        let descriptor = Box::leak(Box::new(
            ServiceDescriptor::new(
                selector(),
                vec![
                    MethodDescriptor::new(
                        MethodId::new(0),
                        "call",
                        HttpOperation::new(
                            "POST".parse().unwrap(),
                            "/call",
                            vec![],
                            "application/json",
                            "application/json",
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        ));
        Arc::new(
            ServiceRegistration::new(
                InstanceId::new("demo-1").unwrap(),
                descriptor,
                endpoint.parse().unwrap(),
                EndpointCapabilities::new(
                    HttpVersionSet::ALL,
                    [
                        HttpBindingId::new("vendor-v1").unwrap(),
                        HttpBindingId::default(),
                    ],
                    true,
                )
                .unwrap(),
                ServiceWeight::new(3.0).unwrap(),
            )
            .with_metadata(std::collections::BTreeMap::from([(
                "zone".into(),
                "east".into(),
            )]))
            .unwrap(),
        )
    }

    fn registration() -> Arc<ServiceRegistration> {
        registration_at("http://127.0.0.1:8080/rpc")
    }

    fn instance(address: &str) -> ServiceInstance {
        ServiceInstance::new(
            InstanceId::new("test-instance").unwrap(),
            address.parse().unwrap(),
            EndpointCapabilities::default(),
            ServiceWeight::default(),
        )
    }

    #[test]
    fn service_name_and_group_are_binding_independent() {
        assert_eq!(service_name(&selector()), "demo");
        assert_eq!(service_group(&selector()).as_deref(), Some("prod"));
        let ungrouped = ServiceSelector::new("demo", None, None).unwrap();
        assert_eq!(service_group(&ungrouped).as_deref(), Some(DEFAULT_GROUP));
    }

    #[test]
    fn registration_metadata_preserves_identity_address_and_weight() {
        let registration = registration();
        let instance = build_instance(&registration).unwrap();
        assert_eq!(instance.instance_id.as_deref(), Some("demo-1"));
        assert_eq!(instance.weight, 3.0);
        assert_eq!(instance.metadata[META_SCHEME], "http");
        assert_eq!(instance.metadata[META_BASE_PATH], "/rpc");
        assert_eq!(
            instance.metadata[META_HTTP_BINDINGS],
            "http-json-v1,vendor-v1"
        );
        assert_eq!(instance.metadata[META_HTTP_VERSIONS], "1.1,2");
        assert_eq!(instance.metadata["zone"], "east");
        assert_eq!(
            instance.metadata[META_INVOCATION_CONTROLS],
            INVOCATION_CONTROLS_V1
        );
        assert!(!instance.metadata.contains_key("fusen.protocol"));

        let tls = registration_at("https://service.example:443/rpc");
        let tls = build_instance(&tls).unwrap();
        assert_eq!(tls.ip, "service.example");
        assert_eq!(tls.port, 443);
        assert_eq!(tls.metadata[META_SCHEME], "https");
    }

    #[test]
    fn discovery_preserves_http_and_https_and_rejects_unknown_schemes() {
        let plaintext = NacosServiceInstance {
            instance_id: Some("provider-1".into()),
            ip: "::1".into(),
            port: 8080,
            weight: 2.0,
            metadata: std::collections::HashMap::from([
                (META_SCHEME.into(), "http".into()),
                (META_BASE_PATH.into(), "/api%20v1/a%2Fb".into()),
                (META_SERVICE_ID.into(), "demo".into()),
                (META_VERSION.into(), "1".into()),
                (META_HTTP_BINDINGS.into(), "http-json-v1".into()),
                (META_HTTP_VERSIONS.into(), "1.1,2".into()),
                ("zone".into(), "east".into()),
            ]),
            ..NacosServiceInstance::default()
        };
        let tls = NacosServiceInstance {
            instance_id: Some("provider-tls".into()),
            ip: "127.0.0.1".into(),
            port: 8443,
            metadata: std::collections::HashMap::from([
                (META_SCHEME.into(), "https".into()),
                (META_SERVICE_ID.into(), "demo".into()),
                (META_VERSION.into(), "1".into()),
                (META_HTTP_BINDINGS.into(), "http-json-v1".into()),
                (META_HTTP_VERSIONS.into(), "1.1".into()),
            ]),
            ..NacosServiceInstance::default()
        };
        let unsupported = NacosServiceInstance {
            ip: "127.0.0.1".into(),
            port: 21,
            metadata: std::collections::HashMap::from([
                (META_SCHEME.into(), "ftp".into()),
                (META_SERVICE_ID.into(), "demo".into()),
                (META_VERSION.into(), "1".into()),
                (META_HTTP_BINDINGS.into(), "http-json-v1".into()),
                (META_HTTP_VERSIONS.into(), "1.1".into()),
            ]),
            ..NacosServiceInstance::default()
        };
        let instances = to_service_instances(
            vec![tls, unsupported, plaintext],
            &selector(),
            NacosConvention::Canonical,
        );
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].instance_id().as_str(), "provider-tls");
        assert_eq!(instances[0].endpoint().as_str(), "https://127.0.0.1:8443/");
        assert_eq!(
            instances[0].capabilities().http_versions(),
            HttpVersionSet::HTTP_1_1
        );
        assert_eq!(instances[1].instance_id().as_str(), "provider-1");
        assert_eq!(
            instances[1].endpoint().as_str(),
            "http://[::1]:8080/api%20v1/a%2Fb"
        );
        assert!(instances.iter().all(|instance| {
            instance
                .metadata()
                .keys()
                .all(|key| !key.starts_with("fusen."))
        }));
        assert_eq!(instances[1].metadata()["zone"], "east");
    }

    #[test]
    fn discovery_filters_versions_and_spring_accepts_external_instances() {
        let external = NacosServiceInstance {
            instance_id: Some("external".into()),
            ip: "127.0.0.1".into(),
            port: 8080,
            ..NacosServiceInstance::default()
        };
        let unversioned = ServiceSelector::new("demo", None, None).unwrap();
        assert!(
            to_service_instances(
                vec![external.clone()],
                &unversioned,
                NacosConvention::Canonical,
            )
            .is_empty()
        );
        assert_eq!(
            to_service_instances(
                vec![external.clone()],
                &unversioned,
                NacosConvention::SpringCloud,
            )
            .len(),
            1
        );
        let spring = to_service_instances(
            vec![external.clone()],
            &unversioned,
            NacosConvention::SpringCloud,
        );
        assert_eq!(spring[0].capabilities(), &EndpointCapabilities::default());
        assert!(
            to_service_instances(vec![external], &selector(), NacosConvention::SpringCloud,)
                .is_empty()
        );
    }

    #[test]
    fn capability_metadata_rejects_partial_duplicate_and_unknown_values() {
        let valid = std::collections::HashMap::from([
            (META_HTTP_BINDINGS.into(), "http-json-v1,vendor-v1".into()),
            (META_HTTP_VERSIONS.into(), "2,1.1".into()),
            (
                META_INVOCATION_CONTROLS.into(),
                INVOCATION_CONTROLS_V1.into(),
            ),
        ]);
        let capabilities = decode_capabilities(&valid, NacosConvention::Canonical).unwrap();
        assert_eq!(capabilities.http_versions(), HttpVersionSet::ALL);
        assert!(capabilities.invocation_controls());

        for metadata in [
            std::collections::HashMap::from([(META_HTTP_BINDINGS.into(), "http-json-v1".into())]),
            std::collections::HashMap::from([
                (
                    META_HTTP_BINDINGS.into(),
                    "http-json-v1,http-json-v1".into(),
                ),
                (META_HTTP_VERSIONS.into(), "1.1".into()),
            ]),
            std::collections::HashMap::from([
                (META_HTTP_BINDINGS.into(), "HTTP-JSON-V1".into()),
                (META_HTTP_VERSIONS.into(), "1.1".into()),
            ]),
            std::collections::HashMap::from([
                (META_HTTP_BINDINGS.into(), String::new()),
                (META_HTTP_VERSIONS.into(), "1.1".into()),
            ]),
            std::collections::HashMap::from([
                (META_HTTP_BINDINGS.into(), "http-json-v1".into()),
                (META_HTTP_VERSIONS.into(), "1.1,1.1".into()),
            ]),
            std::collections::HashMap::from([
                (META_HTTP_BINDINGS.into(), "http-json-v1".into()),
                (META_HTTP_VERSIONS.into(), String::new()),
            ]),
            std::collections::HashMap::from([
                (META_HTTP_BINDINGS.into(), "http-json-v1".into()),
                (META_HTTP_VERSIONS.into(), "3".into()),
            ]),
            std::collections::HashMap::from([
                (META_HTTP_BINDINGS.into(), "http-json-v1".into()),
                (META_HTTP_VERSIONS.into(), "1.1".into()),
                (META_INVOCATION_CONTROLS.into(), "true".into()),
            ]),
        ] {
            assert!(
                decode_capabilities(&metadata, NacosConvention::SpringCloud).is_none(),
                "{metadata:?}"
            );
        }
    }

    #[test]
    fn initialization_event_wins_over_an_older_fetch() {
        let (publisher, directory) = directory();
        let gate = SnapshotGate::new(publisher);
        gate.update(vec![instance("http://127.0.0.1:9002")])
            .unwrap();
        gate.initialize(vec![instance("http://127.0.0.1:9001")])
            .unwrap();
        assert_eq!(
            directory.snapshot()[0].endpoint().as_str(),
            "http://127.0.0.1:9002/"
        );
    }

    #[derive(Clone)]
    struct BlockingNaming {
        started: Arc<Notify>,
        release: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
        registrations: Arc<AtomicUsize>,
        deregistrations: Arc<AtomicUsize>,
    }

    impl NamingOperations for BlockingNaming {
        fn register(
            &self,
            _service_name: String,
            _group: Option<String>,
            _instance: NacosServiceInstance,
        ) -> RegistryFuture<()> {
            let provider = self.clone();
            Box::pin(async move {
                provider.registrations.fetch_add(1, Ordering::SeqCst);
                provider.started.notify_one();
                let receiver = provider.release.lock().unwrap().take().unwrap();
                let _ = receiver.await;
                Ok(())
            })
        }

        fn deregister(
            &self,
            _service_name: String,
            _group: Option<String>,
            _instance: NacosServiceInstance,
        ) -> RegistryFuture<()> {
            let deregistrations = self.deregistrations.clone();
            Box::pin(async move {
                deregistrations.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }

        fn subscribe(
            &self,
            _service_name: String,
            _group: Option<String>,
            _clusters: Vec<String>,
            _listener: Arc<dyn NamingEventListener>,
        ) -> RegistryFuture<()> {
            Box::pin(async {
                Err(RegistryError::message(
                    RegistryOperation::ActivateSubscription,
                    RegistryErrorKind::Internal,
                    "not used by registration test",
                ))
            })
        }

        fn unsubscribe(
            &self,
            _service_name: String,
            _group: Option<String>,
            _clusters: Vec<String>,
            _listener: Arc<dyn NamingEventListener>,
        ) -> RegistryFuture<()> {
            Box::pin(async { Ok(()) })
        }

        fn select(
            &self,
            _service_name: String,
            _group: Option<String>,
            _clusters: Vec<String>,
        ) -> RegistryFuture<Vec<NacosServiceInstance>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    #[tokio::test]
    async fn cancelled_nacos_activation_waiter_is_compensated_once() {
        let (release_sender, release_receiver) = oneshot::channel();
        let provider = BlockingNaming {
            started: Arc::new(Notify::new()),
            release: Arc::new(Mutex::new(Some(release_receiver))),
            registrations: Arc::new(AtomicUsize::new(0)),
            deregistrations: Arc::new(AtomicUsize::new(0)),
        };
        let registry = NacosRegistry {
            naming: Arc::new(provider.clone()),
            convention: NacosConvention::Canonical,
        };
        let handle = registry
            .prepare_registration(RegistrationRequest::new(registration()))
            .unwrap();
        let waiter = tokio::spawn({
            let handle = handle.clone();
            async move { handle.activate().await }
        });
        provider.started.notified().await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        let closing = tokio::spawn({
            let handle = handle.clone();
            async move { handle.close().await }
        });
        release_sender.send(()).unwrap();
        closing.await.unwrap().unwrap();
        assert_eq!(provider.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(provider.deregistrations.load(Ordering::SeqCst), 1);
    }
}
