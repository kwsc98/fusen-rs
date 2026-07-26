//! Compile-time consumer proving that generated code honors a renamed `fusen-rs` dependency.

use rpc::{RpcError, service};

#[service(name = "renamed", group = "test", version = "1")]
/// Service contract used to prove dependency-renamed macro expansion.
pub trait RenamedRuntimeService {
    #[rpc::method(
        idempotency = "safe",
        spring(method = "GET", path = "/renamed/{id}", query = ["expand"])
    )]
    /// Looks up one value through both generated protocol mappings.
    async fn lookup(&self, id: String, expand: Option<bool>) -> Result<String, RpcError>;
}

/// Minimal direct implementation used by the renamed-runtime consumer test.
pub struct Service;

impl RenamedRuntimeService for Service {
    async fn lookup(&self, id: String, _expand: Option<bool>) -> Result<String, RpcError> {
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renamed_runtime_and_direct_trait_implementation_compile() {
        let descriptor = RenamedRuntimeServiceClient::descriptor();
        assert_eq!(descriptor.selector().service_id(), "renamed");
        assert_eq!(descriptor.selector().group(), Some("test"));
        assert_eq!(descriptor.selector().version(), Some("1"));
        assert_eq!(descriptor.methods()[0].fusen_identity(), "lookup");
        let spring = descriptor.methods()[0].spring_cloud().unwrap();
        assert_eq!(spring.method().as_str(), "GET");
        assert_eq!(spring.path(), "/renamed/{id}");

        let _server = RenamedRuntimeServiceServer::new(Service);
    }
}
