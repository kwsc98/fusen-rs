use crate::NacosConfig;
use fusen_config::Error;
use fusen_register::{
    Register, ServiceSubscription, SubscriptionCloser,
    contract::{
        ServiceEndpoint, ServiceInstance, ServiceRegistration, ServiceSelector, ServiceWeight,
        StaticBoxFuture, WireProtocol,
    },
    directory::{DirectoryWriter, directory_channel},
    error::RegisterError,
    subscription_cleanup,
};
use nacos_sdk::api::{
    naming::{
        NamingChangeEvent, NamingEventListener, NamingService, NamingServiceBuilder,
        ServiceInstance as NacosServiceInstance,
    },
    props::ClientProps,
};
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;

const META_SCHEME: &str = "fusen.scheme";
const META_BASE_PATH: &str = "fusen.base_path";
const META_SERVICE_ID: &str = "fusen.service_id";
const META_VERSION: &str = "fusen.version";
const META_GROUP: &str = "fusen.group";

#[derive(Clone)]
/// Nacos-backed service registry and discovery provider.
pub struct NacosRegister {
    naming_service: Arc<NamingService>,
}

impl NacosRegister {
    /// Connects the Nacos naming client for one application.
    pub async fn init_nacos_register(
        app_name: &str,
        config: Arc<NacosConfig>,
    ) -> Result<Self, Error> {
        let props = ClientProps::new()
            .server_addr(config.server_addr.clone())
            .namespace(config.namespace.clone().unwrap_or_default())
            .app_name(app_name)
            .auth_username(config.username.clone().unwrap_or_default())
            .auth_password(config.password.clone().unwrap_or_default());
        let builder = NamingServiceBuilder::new(props);
        let builder = if config.username.is_some() {
            builder.enable_auth_plugin_http()
        } else {
            builder
        };
        Ok(Self {
            naming_service: Arc::new(builder.build().await.map_err(Error::register)?),
        })
    }
}

impl Register for NacosRegister {
    fn register(
        &self,
        registration: Arc<ServiceRegistration>,
        protocol: WireProtocol,
    ) -> StaticBoxFuture<Result<(), RegisterError>> {
        let naming = self.naming_service.clone();
        Box::pin(async move {
            let service_name = get_service_name(registration.selector(), protocol)?;
            let instance = build_instance(&registration)?;
            let group = registration.selector().group().map(str::to_owned);
            let register_naming = naming.clone();
            let register_service_name = service_name.clone();
            let register_group = group.clone();
            let register_instance = instance.clone();
            cancellation_safe_registration(
                async move {
                    register_naming
                        .register_instance(register_service_name, register_group, register_instance)
                        .await
                        .map_err(RegisterError::provider)
                },
                move || async move {
                    naming
                        .deregister_instance(service_name, group, instance)
                        .await
                        .map_err(RegisterError::provider)
                },
            )
            .await
        })
    }

    fn deregister(
        &self,
        registration: Arc<ServiceRegistration>,
        protocol: WireProtocol,
    ) -> StaticBoxFuture<Result<(), RegisterError>> {
        let naming = self.naming_service.clone();
        Box::pin(async move {
            let service_name = get_service_name(registration.selector(), protocol)?;
            let instance = build_instance(&registration)?;
            naming
                .deregister_instance(
                    service_name,
                    registration.selector().group().map(str::to_owned),
                    instance,
                )
                .await
                .map_err(RegisterError::provider)
        })
    }

    fn subscribe(
        &self,
        selector: ServiceSelector,
        protocol: WireProtocol,
    ) -> StaticBoxFuture<Result<ServiceSubscription, RegisterError>> {
        let naming = self.naming_service.clone();
        Box::pin(async move {
            let service_name = get_service_name(&selector, protocol)?;
            let group = selector.group().map(str::to_owned);
            let clusters = Vec::new();
            let (directory_writer, directory) = directory_channel(Vec::new());
            let snapshot_gate = Arc::new(SnapshotGate::new(directory_writer));
            let listener: Arc<dyn NamingEventListener> = Arc::new(ServiceChangeListener {
                snapshot_gate: snapshot_gate.clone(),
            });
            naming
                .subscribe(
                    service_name.clone(),
                    group.clone(),
                    clusters.clone(),
                    listener.clone(),
                )
                .await
                .map_err(RegisterError::provider)?;
            let (closer, cleanup) = subscription_cleanup();
            let cleanup_naming = naming.clone();
            let cleanup_service_name = service_name.clone();
            let cleanup_group = group.clone();
            let cleanup_clusters = clusters.clone();
            tokio::spawn(cleanup.run(async move {
                cleanup_naming
                    .unsubscribe(
                        cleanup_service_name,
                        cleanup_group,
                        cleanup_clusters,
                        listener,
                    )
                    .await
                    .map_err(RegisterError::provider)
            }));
            let setup_guard = NamingSetupGuard::new(closer);
            let instances = naming
                .select_instances(service_name, group, clusters, false, true)
                .await
                .map_err(RegisterError::provider)?;
            snapshot_gate.initialize(to_service_instances(instances))?;
            Ok(ServiceSubscription::new(directory, setup_guard.disarm()))
        })
    }
}

async fn cancellation_safe_registration<R, C, CF>(
    registration: R,
    compensation: C,
) -> Result<(), RegisterError>
where
    R: Future<Output = Result<(), RegisterError>> + Send + 'static,
    C: FnOnce() -> CF + Send + 'static,
    CF: Future<Output = Result<(), RegisterError>> + Send + 'static,
{
    let (result_sender, result_receiver) = oneshot::channel();
    let (acknowledged_sender, acknowledged_receiver) = oneshot::channel();
    tokio::spawn(async move {
        match registration.await {
            Err(error) => {
                let _ = result_sender.send(Err(error));
            }
            Ok(()) => {
                let delivered = result_sender.send(Ok(())).is_ok();
                if delivered && acknowledged_receiver.await.is_ok() {
                    return;
                }
                if let Err(error) = compensation().await {
                    tracing::error!(?error, "late Nacos registration compensation failed");
                }
            }
        }
    });
    let result = result_receiver.await.map_err(|_| {
        RegisterError::provider(std::io::Error::other(
            "Nacos registration task ended without a result",
        ))
    })?;
    if result.is_ok() {
        let _ = acknowledged_sender.send(());
    }
    result
}

struct NamingSetupGuard {
    closer: Option<SubscriptionCloser>,
}

impl NamingSetupGuard {
    fn new(closer: SubscriptionCloser) -> Self {
        Self {
            closer: Some(closer),
        }
    }

    fn disarm(mut self) -> SubscriptionCloser {
        self.closer
            .take()
            .expect("setup closer is present until disarmed")
    }
}

impl Drop for NamingSetupGuard {
    fn drop(&mut self) {
        if let Some(closer) = &self.closer {
            closer.request_close();
        }
    }
}

#[derive(Default)]
struct SnapshotGateState {
    initialized: bool,
    pending: Option<Vec<ServiceInstance>>,
}

struct SnapshotGate {
    writer: DirectoryWriter,
    state: Mutex<SnapshotGateState>,
}

impl SnapshotGate {
    fn new(writer: DirectoryWriter) -> Self {
        Self {
            writer,
            state: Mutex::new(SnapshotGateState::default()),
        }
    }

    fn initialize(&self, initial: Vec<ServiceInstance>) -> Result<(), RegisterError> {
        let mut state = self.state.lock().map_err(|_| {
            RegisterError::provider(std::io::Error::other("Nacos snapshot gate was poisoned"))
        })?;
        let snapshot = state.pending.take().unwrap_or(initial);
        self.writer.replace(snapshot);
        state.initialized = true;
        Ok(())
    }

    fn update(&self, resources: Vec<ServiceInstance>) -> Result<(), RegisterError> {
        let mut state = self.state.lock().map_err(|_| {
            RegisterError::provider(std::io::Error::other("Nacos snapshot gate was poisoned"))
        })?;
        if state.initialized {
            self.writer.replace(resources);
            Ok(())
        } else {
            state.pending = Some(resources);
            Ok(())
        }
    }
}

#[derive(Clone)]
struct ServiceChangeListener {
    snapshot_gate: Arc<SnapshotGate>,
}

impl NamingEventListener for ServiceChangeListener {
    fn event(&self, event: Arc<NamingChangeEvent>) {
        let resources = event
            .instances
            .clone()
            .map(to_service_instances)
            .unwrap_or_default();
        if let Err(error) = self.snapshot_gate.update(resources) {
            tracing::error!(?error, service = %event.service_name, "failed to update service directory");
        }
    }
}

fn to_service_instances(instances: Vec<NacosServiceInstance>) -> Vec<ServiceInstance> {
    instances
        .into_iter()
        .filter(|instance| instance.healthy && instance.enabled && instance.weight > 0.0)
        .filter_map(|instance| {
            let port = u16::try_from(instance.port).ok()?;
            let host = if instance.ip.parse::<std::net::Ipv6Addr>().is_ok() {
                format!("[{}]", instance.ip)
            } else {
                instance.ip.clone()
            };
            let scheme = instance
                .metadata
                .get(META_SCHEME)
                .map(String::as_str)
                .unwrap_or("http");
            if !matches!(scheme, "http" | "https") {
                return None;
            }
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
            let endpoint = ServiceEndpoint::try_from(url).ok()?;
            let weight = ServiceWeight::new(instance.weight).ok()?;
            ServiceInstance::new(endpoint, weight)
                .with_metadata(instance.metadata.into_iter().collect())
                .ok()
        })
        .collect()
}

pub fn get_service_name(
    selector: &ServiceSelector,
    protocol: WireProtocol,
) -> Result<String, RegisterError> {
    match protocol {
        WireProtocol::SpringCloud => Ok(selector
            .metadata()
            .get("spring.application.name")
            .cloned()
            .unwrap_or_else(|| selector.service_id().to_owned())),
        WireProtocol::Fusen => Ok(format!(
            "providers:{}:{}:{}",
            selector.service_id(),
            selector.version().unwrap_or(""),
            selector.group().unwrap_or("")
        )),
        _ => Err(RegisterError::UnsupportedProtocol(protocol.to_string())),
    }
}

fn build_instance(
    registration: &ServiceRegistration,
) -> Result<NacosServiceInstance, RegisterError> {
    let url = registration.endpoint().as_url();
    let ip = url
        .host_str()
        .ok_or_else(|| RegisterError::InvalidResource("advertised URL has no host".into()))?;
    let ip = ip
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(ip);
    let port = url
        .port_or_known_default()
        .ok_or_else(|| RegisterError::InvalidResource("advertised URL has no port".into()))?;
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
        ip: ip.to_owned(),
        port: i32::from(port),
        weight: registration.weight().get(),
        metadata,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusen_register::contract::{MethodDescriptor, MethodId, ServiceDescriptor};
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };
    use tokio::sync::Notify;

    fn selector() -> ServiceSelector {
        ServiceSelector::new("demo", Some("DEFAULT_GROUP".into()), Some("1.0".into())).unwrap()
    }

    fn instance(addr: &str) -> ServiceInstance {
        ServiceInstance::new(addr.parse().unwrap(), ServiceWeight::default())
    }

    fn descriptor(selector: ServiceSelector) -> &'static ServiceDescriptor {
        Box::leak(Box::new(
            ServiceDescriptor::__from_selector(
                selector,
                vec![
                    MethodDescriptor::__new(
                        MethodId::__new(0),
                        "call",
                        http::Method::GET,
                        "/call",
                        Vec::new(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        ))
    }

    #[test]
    fn rejects_invalid_advertised_url() {
        assert!("127.0.0.1:8080".parse::<ServiceEndpoint>().is_err());
    }

    #[test]
    fn spring_cloud_service_name_uses_metadata() {
        let resource = selector()
            .with_metadata(std::collections::BTreeMap::from([(
                "spring.application.name".into(),
                "orders".into(),
            )]))
            .unwrap();
        assert_eq!(
            get_service_name(&resource, WireProtocol::SpringCloud).unwrap(),
            "orders"
        );
    }

    #[test]
    fn filters_unhealthy_instances_and_restores_https_base_path() {
        let healthy = NacosServiceInstance {
            ip: "::1".into(),
            port: 8443,
            weight: 2.0,
            metadata: HashMap::from([
                (META_SCHEME.into(), "https".into()),
                (META_BASE_PATH.into(), "/rpc".into()),
                (META_SERVICE_ID.into(), "demo".into()),
            ]),
            ..NacosServiceInstance::default()
        };
        let unhealthy = NacosServiceInstance {
            healthy: false,
            ip: "127.0.0.1".into(),
            port: 8080,
            ..NacosServiceInstance::default()
        };
        let resources = to_service_instances(vec![unhealthy, healthy]);
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].endpoint().as_str(), "https://[::1]:8443/rpc");
        assert_eq!(resources[0].weight().get(), 2.0);
    }

    #[test]
    fn preserves_encoded_base_path_without_double_encoding() {
        let instance = NacosServiceInstance {
            ip: "127.0.0.1".into(),
            port: 8443,
            metadata: HashMap::from([
                (META_SCHEME.into(), "https".into()),
                (META_BASE_PATH.into(), "/api%20v1/a%2Fb".into()),
                (META_SERVICE_ID.into(), "demo".into()),
            ]),
            ..NacosServiceInstance::default()
        };
        let resources = to_service_instances(vec![instance]);
        assert_eq!(
            resources[0].endpoint().as_str(),
            "https://127.0.0.1:8443/api%20v1/a%2Fb"
        );
    }

    #[test]
    fn ignores_instances_with_non_http_metadata() {
        let instance = NacosServiceInstance {
            ip: "127.0.0.1".into(),
            port: 21,
            metadata: HashMap::from([(META_SCHEME.into(), "ftp".into())]),
            ..NacosServiceInstance::default()
        };
        assert!(to_service_instances(vec![instance]).is_empty());
    }

    #[test]
    fn registration_metadata_preserves_address_and_weight() {
        let descriptor = descriptor(selector());
        let resource = ServiceRegistration::__new(
            descriptor,
            "https://127.0.0.1:8443/rpc".parse().unwrap(),
            ServiceWeight::new(3.0).unwrap(),
        )
        .unwrap();
        let instance = build_instance(&resource).unwrap();
        assert_eq!(instance.weight, 3.0);
        assert_eq!(instance.metadata[META_SCHEME], "https");
        assert_eq!(instance.metadata[META_BASE_PATH], "/rpc");
    }

    #[test]
    fn initialization_event_overrides_stale_snapshot() {
        let (writer, directory) = directory_channel(Vec::new());
        let gate = SnapshotGate::new(writer);
        gate.update(vec![instance("http://127.0.0.1:9002")])
            .unwrap();
        gate.initialize(vec![instance("http://127.0.0.1:9001")])
            .unwrap();
        assert_eq!(
            directory.snapshot()[0].endpoint().as_str(),
            "http://127.0.0.1:9002/"
        );
    }

    #[test]
    fn snapshot_gate_keeps_latest_initialization_event_and_live_updates() {
        let (writer, directory) = directory_channel(Vec::new());
        let gate = SnapshotGate::new(writer);
        gate.update(vec![instance("http://127.0.0.1:9001")])
            .unwrap();
        gate.update(vec![instance("http://127.0.0.1:9002")])
            .unwrap();
        gate.initialize(vec![instance("http://127.0.0.1:9000")])
            .unwrap();
        assert_eq!(
            directory.snapshot()[0].endpoint().as_str(),
            "http://127.0.0.1:9002/"
        );
        gate.update(vec![instance("http://127.0.0.1:9003")])
            .unwrap();
        assert_eq!(
            directory.snapshot()[0].endpoint().as_str(),
            "http://127.0.0.1:9003/"
        );
    }

    #[tokio::test]
    async fn late_registration_success_is_compensated_after_caller_cancellation() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let compensated = Arc::new(Notify::new());
        let compensation_count = Arc::new(AtomicUsize::new(0));
        let operation = tokio::spawn(cancellation_safe_registration(
            {
                let started = started.clone();
                let release = release.clone();
                async move {
                    started.notify_one();
                    release.notified().await;
                    Ok(())
                }
            },
            {
                let compensated = compensated.clone();
                let compensation_count = compensation_count.clone();
                move || async move {
                    compensation_count.fetch_add(1, Ordering::SeqCst);
                    compensated.notify_one();
                    Ok(())
                }
            },
        ));
        started.notified().await;
        operation.abort();
        let _ = operation.await;
        release.notify_waiters();
        tokio::time::timeout(Duration::from_secs(1), compensated.notified())
            .await
            .expect("late registration success was not compensated");
        assert_eq!(compensation_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    #[ignore = "requires NACOS_ADDR and a manually managed Nacos server"]
    async fn live_nacos_discovery_updates_and_cleanup_when_configured() {
        let server_addr = std::env::var("NACOS_ADDR")
            .expect("set NACOS_ADDR before running the ignored Nacos integration test");
        let register = NacosRegister::init_nacos_register(
            "fusen-test",
            Arc::new(NacosConfig {
                server_addr,
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let selector = ServiceSelector::new(
            format!("fusen-live-{unique}"),
            Some("DEFAULT_GROUP".into()),
            Some("1.0".into()),
        )
        .unwrap();
        let resource = |address: &str| {
            Arc::new(
                ServiceRegistration::__new(
                    descriptor(selector.clone()),
                    address.parse().unwrap(),
                    ServiceWeight::default(),
                )
                .unwrap(),
            )
        };
        let first = resource("http://127.0.0.1:18081");
        let second = resource("http://127.0.0.1:18082");
        let subscription = register
            .subscribe(selector, WireProtocol::Fusen)
            .await
            .unwrap();

        register
            .register(first.clone(), WireProtocol::Fusen)
            .await
            .unwrap();
        wait_for_snapshot(&subscription, |instances| {
            instances
                .iter()
                .any(|instance| instance.endpoint() == first.endpoint())
        })
        .await;

        register
            .register(second.clone(), WireProtocol::Fusen)
            .await
            .unwrap();
        wait_for_snapshot(&subscription, |instances| {
            instances
                .iter()
                .any(|instance| instance.endpoint() == first.endpoint())
                && instances
                    .iter()
                    .any(|instance| instance.endpoint() == second.endpoint())
        })
        .await;

        register
            .deregister(first.clone(), WireProtocol::Fusen)
            .await
            .unwrap();
        wait_for_snapshot(&subscription, |instances| {
            !instances
                .iter()
                .any(|instance| instance.endpoint() == first.endpoint())
                && instances
                    .iter()
                    .any(|instance| instance.endpoint() == second.endpoint())
        })
        .await;

        subscription.close().await.unwrap();
        register
            .deregister(second, WireProtocol::Fusen)
            .await
            .unwrap();
    }

    async fn wait_for_snapshot(
        subscription: &ServiceSubscription,
        predicate: impl Fn(&[Arc<ServiceInstance>]) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let snapshot = subscription.directory().snapshot();
                if predicate(&snapshot) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("Nacos discovery snapshot did not update before the deadline");
    }
}
