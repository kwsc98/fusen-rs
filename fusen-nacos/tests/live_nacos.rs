//! Live Nacos release-gate coverage for registration, discovery, config, and cleanup.

use fusen_config::{ConfigHandle, ConfigKey, ConfigSource};
use fusen_contract::{
    EndpointCapabilities, HttpOperation, InstanceId, MethodDescriptor, MethodId, ServiceDescriptor,
    ServiceRegistration, ServiceSelector, ServiceWeight,
};
use fusen_nacos::{NacosConfig, NacosConfigSource, NacosRegistry};
use fusen_register::{RegistrationRequest, Registry, SubscriptionRequest, directory::Directory};
use nacos_sdk::api::{
    config::{ConfigService, ConfigServiceBuilder},
    error::Error as NacosError,
    naming::{NamingService, NamingServiceBuilder},
    props::ClientProps,
};
use serde::Deserialize;
use std::{
    env,
    error::Error,
    fmt::Display,
    future::Future,
    io,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type BoxError = Box<dyn Error + Send + Sync>;
type TestResult<T = ()> = Result<T, BoxError>;

const GROUP: &str = "fusen_release_gate";
const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_POLL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct LiveSettings {
    value: String,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the release-gate Nacos container"]
async fn live_nacos_registration_discovery_config_and_cleanup() -> TestResult {
    let server_addr = env::var("NACOS_ADDR")
        .map_err(|_| failure("NACOS_ADDR must point at the release-gate Nacos container"))?;
    let resource = unique_resource()?;
    let adapter_config = NacosConfig::builder()
        .server_addr(server_addr.clone())
        .build()?;
    let admin_props = ClientProps::new()
        .env_first(false)
        .server_addr(server_addr)
        .namespace("")
        .app_name(format!("{resource}-admin"));

    live_registry_case(&resource, adapter_config.clone(), admin_props.clone()).await?;
    live_config_case(&resource, adapter_config, admin_props).await
}

async fn live_registry_case(
    resource: &str,
    config: NacosConfig,
    admin_props: ClientProps,
) -> TestResult {
    let selector = ServiceSelector::new(resource, Some(GROUP.into()), Some("1".into()))?;
    let descriptor = Box::leak(Box::new(ServiceDescriptor::new(
        selector.clone(),
        vec![MethodDescriptor::new(
            MethodId::new(0),
            "ping",
            HttpOperation::new(
                "GET".parse()?,
                "/ping",
                vec![],
                "application/json",
                "application/json",
            )?,
        )?],
    )?));
    let instance_id = InstanceId::new(format!("{resource}-instance"))?;
    let registration = Arc::new(ServiceRegistration::new(
        instance_id.clone(),
        descriptor,
        "http://127.0.0.1:18080/live".parse()?,
        EndpointCapabilities::default(),
        ServiceWeight::default(),
    ));
    let registry = timed(
        "connect Nacos registry adapter",
        NacosRegistry::connect(format!("{resource}-registry"), config),
    )
    .await?;
    let admin = timed(
        "connect Nacos naming cleanup client",
        NamingServiceBuilder::new(admin_props).build(),
    )
    .await?;
    let subscription = registry.prepare_subscription(SubscriptionRequest::new(selector))?;
    let registration_handle =
        registry.prepare_registration(RegistrationRequest::new(registration))?;

    let exercise = async {
        let directory = timed("activate Nacos subscription", subscription.activate()).await?;
        timed(
            "activate Nacos registration",
            registration_handle.activate(),
        )
        .await?;
        wait_for_directory_instance(directory, &instance_id).await
    }
    .await;

    let registration_cleanup = timed("close Nacos registration", registration_handle.close()).await;
    let service_name = resource.to_owned();
    let remote_cleanup =
        wait_for_remote_instance_absence(&admin, &service_name, Some(GROUP.into()), &instance_id)
            .await;
    let subscription_cleanup = timed("close Nacos subscription", subscription.close()).await;

    finish_with_cleanup(
        exercise,
        [
            ("registration close", registration_cleanup),
            ("remote registration", remote_cleanup),
            ("subscription close", subscription_cleanup),
        ],
    )
}

async fn wait_for_directory_instance(
    mut directory: Directory,
    instance_id: &InstanceId,
) -> TestResult {
    timed(
        "observe registered instance through Nacos subscription",
        async {
            loop {
                let snapshot = directory.snapshot();
                if snapshot
                    .instances()
                    .iter()
                    .any(|instance| instance.instance_id() == instance_id)
                {
                    return Ok::<_, BoxError>(());
                }
                directory.changed().await?;
            }
        },
    )
    .await
}

async fn wait_for_remote_instance_absence(
    naming: &NamingService,
    service_name: &str,
    group: Option<String>,
    instance_id: &InstanceId,
) -> TestResult {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    loop {
        let instances = timed(
            "query Nacos registration cleanup",
            naming.get_all_instances(service_name.to_owned(), group.clone(), Vec::new(), false),
        )
        .await?;
        if !instances
            .iter()
            .any(|instance| instance.instance_id.as_deref() == Some(instance_id.as_str()))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(failure(format!(
                "Nacos registration {} remained after close",
                instance_id.as_str()
            )));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn live_config_case(
    resource: &str,
    config: NacosConfig,
    admin_props: ClientProps,
) -> TestResult {
    let data_id = format!("{resource}.toml");
    let admin = timed(
        "connect Nacos config cleanup client",
        ConfigServiceBuilder::new(admin_props).build(),
    )
    .await?;
    let initial_publish = timed(
        "publish initial Nacos config",
        admin.publish_config(
            data_id.clone(),
            GROUP.into(),
            "value = \"initial\"".into(),
            Some("toml".into()),
        ),
    )
    .await;

    let mut prepared_handle: Option<ConfigHandle> = None;
    let exercise = match initial_publish {
        Ok(true) => {
            async {
                let source = timed(
                    "connect Nacos config adapter",
                    NacosConfigSource::connect(format!("{resource}-config"), config),
                )
                .await?;
                let handle =
                    source.prepare(ConfigKey::builder(data_id.clone()).group(GROUP).build()?)?;
                prepared_handle = Some(handle.clone());
                timed("activate Nacos config listener", handle.activate()).await?;
                let mut hot = handle.typed::<LiveSettings>()?;
                if hot.current().as_ref()
                    != &(LiveSettings {
                        value: "initial".into(),
                    })
                {
                    return Err(failure(
                        "Nacos config adapter returned the wrong initial value",
                    ));
                }

                let updated = timed(
                    "publish updated Nacos config",
                    admin.publish_config(
                        data_id.clone(),
                        GROUP.into(),
                        "value = \"updated\"".into(),
                        Some("toml".into()),
                    ),
                )
                .await?;
                if !updated {
                    return Err(failure("Nacos rejected the live config update"));
                }
                timed("observe Nacos config update", async {
                    loop {
                        let snapshot = hot.changed().await?;
                        if snapshot.value().value == "updated" {
                            return Ok::<_, BoxError>(());
                        }
                    }
                })
                .await
            }
            .await
        }
        Ok(false) => Err(failure(
            "Nacos rejected the initial live config publication",
        )),
        Err(error) => Err(error),
    };

    let listener_cleanup = match prepared_handle {
        Some(handle) => timed("close Nacos config listener", handle.close()).await,
        None => Ok(()),
    };
    let config_cleanup = remove_and_verify_config(&admin, data_id, GROUP.into()).await;
    finish_with_cleanup(
        exercise,
        [
            ("config listener close", listener_cleanup),
            ("remote config removal", config_cleanup),
        ],
    )
}

async fn remove_and_verify_config(
    admin: &ConfigService,
    data_id: String,
    group: String,
) -> TestResult {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    loop {
        let remove_error = match tokio::time::timeout(
            CLEANUP_POLL_TIMEOUT,
            admin.remove_config(data_id.clone(), group.clone()),
        )
        .await
        {
            Ok(Ok(_)) => None,
            Ok(Err(error)) => Some(format!("remove config failed: {error}")),
            Err(_) => Some("remove config operation timed out".to_owned()),
        };
        let verification_error = match tokio::time::timeout(
            CLEANUP_POLL_TIMEOUT,
            admin.get_config(data_id.clone(), group.clone()),
        )
        .await
        {
            Ok(Err(NacosError::ConfigNotFound(_))) => return Ok(()),
            Ok(Ok(_)) => "config remained readable after removal".to_owned(),
            Ok(Err(error)) => format!("verify config removal failed: {error}"),
            Err(_) => "verify config removal timed out".to_owned(),
        };
        let last_error = remove_error
            .map(|remove_error| format!("{remove_error}; {verification_error}"))
            .unwrap_or(verification_error);
        if Instant::now() >= deadline {
            return Err(failure(format!(
                "Nacos config cleanup did not converge: {last_error}"
            )));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn timed<T, E, F>(label: &str, future: F) -> TestResult<T>
where
    E: Display,
    F: Future<Output = Result<T, E>>,
{
    match tokio::time::timeout(OPERATION_TIMEOUT, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(failure(format!("{label} failed: {error}"))),
        Err(_) => Err(failure(format!("{label} exceeded {OPERATION_TIMEOUT:?}"))),
    }
}

fn finish_with_cleanup<const N: usize>(
    exercise: TestResult,
    cleanup: [(&str, TestResult); N],
) -> TestResult {
    let cleanup_errors = cleanup
        .into_iter()
        .filter_map(|(label, result)| result.err().map(|error| format!("{label}: {error}")))
        .collect::<Vec<_>>();
    match (exercise, cleanup_errors.is_empty()) {
        (Ok(()), true) => Ok(()),
        (Err(error), true) => Err(error),
        (Ok(()), false) => Err(failure(cleanup_errors.join("; "))),
        (Err(error), false) => Err(failure(format!(
            "live exercise failed: {error}; cleanup also failed: {}",
            cleanup_errors.join("; ")
        ))),
    }
}

fn unique_resource() -> TestResult<String> {
    let run_id = env::var("NACOS_TEST_RUN_ID").unwrap_or_else(|_| "local".into());
    let run_id = run_id
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .take(32)
        .map(char::from)
        .collect::<String>();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| failure(format!("system clock precedes UNIX epoch: {error}")))?
        .as_nanos();
    Ok(format!(
        "fusenlive{run_id}{}{timestamp}",
        std::process::id()
    ))
}

fn failure(message: impl Into<String>) -> BoxError {
    Box::new(io::Error::other(message.into()))
}
