//! Compile-time consumer proving that generated code honors a renamed `fusen-rs` dependency.

use rpc::{RpcError, RpcRequest, RpcResponse, interface};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, rpc::RpcMessage)]
/// Request used to verify path and optional query macro expansion.
pub struct LookupRequest {
    /// Required route identifier.
    #[rpc(path)]
    pub id: String,
    /// Optional expansion flag.
    #[rpc(query)]
    pub expand: Option<bool>,
}

#[interface(name = "renamed", group = "test", version = "1")]
/// Interface contract used to prove dependency-renamed macro expansion.
pub trait RenamedRuntimeApi {
    #[rpc::method(idempotency = "safe", spring(method = "GET", path = "/renamed/{id}"))]
    /// Looks up one value through both generated protocol mappings.
    async fn lookup(
        &self,
        request: RpcRequest<LookupRequest>,
    ) -> Result<RpcResponse<String>, RpcError>;
}

/// Minimal direct implementation used by the renamed-runtime consumer test.
pub struct Handler;

impl RenamedRuntimeApi for Handler {
    async fn lookup(
        &self,
        request: RpcRequest<LookupRequest>,
    ) -> Result<RpcResponse<String>, RpcError> {
        Ok(RpcResponse::new(request.into_body().id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renamed_runtime_and_shared_trait_compile() {
        fn accepts_interface<T: RenamedRuntimeApi>() {}
        accepts_interface::<RenamedRuntimeApiClient>();
        accepts_interface::<Handler>();

        let descriptor = RenamedRuntimeApiClient::descriptor().unwrap();
        assert_eq!(descriptor.selector().service_id(), "renamed");
        assert_eq!(descriptor.selector().group(), Some("test"));
        assert_eq!(descriptor.selector().version(), Some("1"));
        assert_eq!(descriptor.methods()[0].fusen_identity(), "lookup");
        let spring = descriptor.methods()[0].spring_cloud().unwrap();
        assert_eq!(spring.method().as_str(), "GET");
        assert_eq!(spring.path(), "/renamed/{id}");

        let _server = RenamedRuntimeApiServer::new(Handler);
    }
}
