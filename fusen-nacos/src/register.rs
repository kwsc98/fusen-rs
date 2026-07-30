use crate::{NacosConfig, client_props, validate_application_name};
use fusen_contract::{
    InstanceId, ServiceEndpoint, ServiceInstance, ServiceRegistration, ServiceSelector,
    ServiceWeight, WireProtocol,
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
const META_PROTOCOL: &str = "fusen.protocol";

/// Nacos-backed service registry and discovery provider.
#[derive(Clone)]
pub struct NacosRegistry {
    naming: Arc<dyn NamingOperations>,
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
        })
    }
}

impl std::fmt::Debug for NacosRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NacosRegistry")
            .finish_non_exhaustive()
    }
}

impl Registry for NacosRegistry {
    fn prepare_registration(
        &self,
        request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError> {
        let (registration, protocol) = request.into_parts();
        if !registration.protocols().contains(protocol) {
            return Err(RegistryError::message(
                RegistryOperation::PrepareRegistration,
                RegistryErrorKind::InvalidResource,
                format!(
                    "registration {} does not advertise {protocol}",
                    registration.selector().identity()
                ),
            ));
        }
        let service_name = service_name(
            registration.selector(),
            protocol,
            RegistryOperation::PrepareRegistration,
        )?;
        let group = registration.selector().group().map(str::to_owned);
        let instance = build_instance(&registration, protocol)?;
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
        let (selector, protocol) = request.into_parts();
        let service_name =
            service_name(&selector, protocol, RegistryOperation::PrepareSubscription)?;
        let group = selector.group().map(str::to_owned);
        let clusters = Vec::new();
        let (publisher, directory) = directory();
        let snapshot_gate = Arc::new(SnapshotGate::new(publisher));
        let listener: Arc<dyn NamingEventListener> = Arc::new(ServiceChangeListener {
            snapshot_gate: snapshot_gate.clone(),
        });

        let activate_naming = self.naming.clone();
        let activate_service_name = service_name.clone();
        let activate_group = group.clone();
        let activate_clusters = clusters.clone();
        let activate_listener = listener.clone();
        let close_naming = self.naming.clone();

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
                snapshot_gate.initialize(to_service_instances(instances))
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
}

impl NamingEventListener for ServiceChangeListener {
    fn event(&self, event: Arc<NamingChangeEvent>) {
        let instances = event
            .instances
            .clone()
            .map(to_service_instances)
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

fn to_service_instances(instances: Vec<NacosServiceInstance>) -> Vec<ServiceInstance> {
    instances
        .into_iter()
        .filter(|instance| instance.healthy && instance.enabled && instance.weight > 0.0)
        .filter_map(to_service_instance)
        .collect()
}

fn to_service_instance(instance: NacosServiceInstance) -> Option<ServiceInstance> {
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
    let weight = ServiceWeight::new(instance.weight).ok()?;
    ServiceInstance::new(instance_id, endpoint, weight)
        .with_metadata(instance.metadata.into_iter().collect())
        .ok()
}

fn service_name(
    selector: &ServiceSelector,
    protocol: WireProtocol,
    operation: RegistryOperation,
) -> Result<String, RegistryError> {
    match protocol {
        WireProtocol::SpringCloudV1 => Ok(selector
            .metadata()
            .get("spring.application.name")
            .cloned()
            .unwrap_or_else(|| selector.service_id().to_owned())),
        WireProtocol::FusenV1 => Ok(format!(
            "fusen:v1:{}:{}:{}",
            selector.service_id(),
            selector.version().unwrap_or(""),
            selector.group().unwrap_or("")
        )),
        _ => Err(RegistryError::message(
            operation,
            RegistryErrorKind::InvalidResource,
            format!("unsupported Nacos wire protocol {protocol}"),
        )),
    }
}

fn build_instance(
    registration: &ServiceRegistration,
    protocol: WireProtocol,
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
    metadata.insert(META_SCHEME.into(), url.scheme().into());
    metadata.insert(
        META_SERVICE_ID.into(),
        registration.selector().service_id().to_owned(),
    );
    metadata.insert(
        META_INSTANCE_ID.into(),
        registration.instance_id().to_string(),
    );
    metadata.insert(META_PROTOCOL.into(), protocol.as_str().into());
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

#[cfg(test)]
mod tests {
    use super::*;
    use fusen_contract::{Idempotency, MethodDescriptor, MethodId, ProtocolSet, ServiceDescriptor};
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
                    MethodDescriptor::new(MethodId::new(0), "call", Idempotency::None, None)
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
                ProtocolSet::FUSEN_V1,
                ServiceWeight::new(3.0).unwrap(),
            )
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
            ServiceWeight::default(),
        )
    }

    #[test]
    fn protocol_names_are_explicitly_versioned() {
        assert_eq!(
            service_name(
                &selector(),
                WireProtocol::FusenV1,
                RegistryOperation::PrepareRegistration,
            )
            .unwrap(),
            "fusen:v1:demo:1:prod"
        );
        let spring = selector()
            .with_metadata(std::collections::BTreeMap::from([(
                "spring.application.name".into(),
                "orders".into(),
            )]))
            .unwrap();
        assert_eq!(
            service_name(
                &spring,
                WireProtocol::SpringCloudV1,
                RegistryOperation::PrepareSubscription,
            )
            .unwrap(),
            "orders"
        );
    }

    #[test]
    fn registration_metadata_preserves_identity_address_and_weight() {
        let registration = registration();
        let instance = build_instance(&registration, WireProtocol::FusenV1).unwrap();
        assert_eq!(instance.instance_id.as_deref(), Some("demo-1"));
        assert_eq!(instance.weight, 3.0);
        assert_eq!(instance.metadata[META_SCHEME], "http");
        assert_eq!(instance.metadata[META_BASE_PATH], "/rpc");
        assert_eq!(instance.metadata[META_PROTOCOL], "fusen-v1");

        let tls = registration_at("https://service.example:443/rpc");
        let tls = build_instance(&tls, WireProtocol::FusenV1).unwrap();
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
            ]),
            ..NacosServiceInstance::default()
        };
        let tls = NacosServiceInstance {
            instance_id: Some("provider-tls".into()),
            ip: "127.0.0.1".into(),
            port: 8443,
            metadata: std::collections::HashMap::from([(META_SCHEME.into(), "https".into())]),
            ..NacosServiceInstance::default()
        };
        let unsupported = NacosServiceInstance {
            ip: "127.0.0.1".into(),
            port: 21,
            metadata: std::collections::HashMap::from([(META_SCHEME.into(), "ftp".into())]),
            ..NacosServiceInstance::default()
        };
        let instances = to_service_instances(vec![tls, unsupported, plaintext]);
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].instance_id().as_str(), "provider-tls");
        assert_eq!(instances[0].endpoint().as_str(), "https://127.0.0.1:8443/");
        assert_eq!(instances[1].instance_id().as_str(), "provider-1");
        assert_eq!(
            instances[1].endpoint().as_str(),
            "http://[::1]:8080/api%20v1/a%2Fb"
        );
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
        };
        let handle = registry
            .prepare_registration(RegistrationRequest::new(
                registration(),
                WireProtocol::FusenV1,
            ))
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
