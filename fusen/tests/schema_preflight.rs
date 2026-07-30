//! Cross-type interface schema failures must be classified before provider or socket I/O.

use fusen_rs::{
    ClientErrorKind, ClientRuntime, RpcError, RpcRequest, RpcResponse, Server, ServerConfig,
    ServerErrorKind, WireProtocol,
    contract::ProtocolSet,
    interface,
    registry::{
        RegistrationHandle, RegistrationRequest, Registry, SubscriptionHandle, SubscriptionRequest,
        error::{RegistryError, RegistryErrorKind, RegistryOperation},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Serialize, Deserialize, fusen_rs::RpcMessage)]
struct InvalidPathRequest {
    #[rpc(path)]
    id: String,
}

#[interface(name = "invalid-schema")]
trait InvalidSchema {
    #[fusen_rs::method(
        idempotency = "safe",
        spring(method = "GET", path = "/items/{missing}")
    )]
    async fn get(
        &self,
        request: RpcRequest<InvalidPathRequest>,
    ) -> Result<RpcResponse<String>, RpcError>;
}

struct InvalidSchemaHandler;

impl InvalidSchema for InvalidSchemaHandler {
    async fn get(
        &self,
        request: RpcRequest<InvalidPathRequest>,
    ) -> Result<RpcResponse<String>, RpcError> {
        Ok(RpcResponse::new(request.into_body().id))
    }
}

struct CountingRegistry {
    prepares: Arc<AtomicUsize>,
}

impl Registry for CountingRegistry {
    fn prepare_registration(
        &self,
        _request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        Err(not_called_error(RegistryOperation::PrepareRegistration))
    }

    fn prepare_subscription(
        &self,
        _request: SubscriptionRequest,
    ) -> Result<SubscriptionHandle, RegistryError> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        Err(not_called_error(RegistryOperation::PrepareSubscription))
    }
}

fn not_called_error(operation: RegistryOperation) -> RegistryError {
    RegistryError::message(
        operation,
        RegistryErrorKind::Internal,
        "schema validation must run before registry preparation",
    )
}

#[tokio::test]
async fn client_connect_rejects_schema_before_discovery_provider_work() {
    let prepares = Arc::new(AtomicUsize::new(0));
    let runtime = ClientRuntime::builder()
        .registry(CountingRegistry {
            prepares: prepares.clone(),
        })
        .build()
        .unwrap();

    let error = InvalidSchemaClient::builder(&runtime)
        .discover()
        .protocol(WireProtocol::SpringCloudV1)
        .connect()
        .await
        .err()
        .expect("invalid schema must reject client connection");
    assert_eq!(error.kind(), ClientErrorKind::Connect);
    assert!(error.message_ref().contains("invalid interface schema"));
    assert_eq!(prepares.load(Ordering::SeqCst), 0);

    runtime.shutdown().await.unwrap();
}

#[test]
fn server_build_rejects_schema_without_binding_the_socket() {
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = probe.local_addr().unwrap();
    drop(probe);

    let error = Server::builder(address.to_string())
        .config(
            ServerConfig::builder()
                .protocols(ProtocolSet::SPRING_CLOUD_V1)
                .build()
                .unwrap(),
        )
        .interface(InvalidSchemaServer::new(InvalidSchemaHandler))
        .build()
        .err()
        .expect("invalid schema must reject server construction");
    assert_eq!(error.kind(), ServerErrorKind::Validation);
    assert!(error.message_ref().contains("invalid interface schema"));

    let rebound = TcpListener::bind(address).expect("Server::build must not bind the socket");
    drop(rebound);
}
