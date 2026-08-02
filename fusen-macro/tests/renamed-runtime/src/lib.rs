//! Compile-time consumer proving that generated code honors a renamed `fusen-rs` dependency.

use runtime::{Error, Response, interface};

#[interface(name = "renamed", group = "test", version = "1")]
/// Interface contract used to prove dependency-renamed macro expansion.
pub trait RenamedRuntimeApi {
    #[runtime::method(method = "GET", path = "/renamed/{id}")]
    /// Looks up one value through the generated HTTP binding.
    async fn lookup(
        &self,
        #[param(context)] call: runtime::Call,
        id: String,
        #[param(query)] expand: Option<bool>,
    ) -> Result<Response<String>, Error>;

    #[runtime::method(method = "POST", path = "/renamed/bindings")]
    /// Exercises generated bindings whose source names overlap with framework internals.
    async fn bindings(
        &self,
        arguments: String,
        handler: String,
        invocation: String,
        method_id: String,
        response: String,
    ) -> Result<runtime::Response<String>, runtime::Error>;

    #[runtime::method(method = "GET", path = "/renamed/raw/{type}")]
    /// Exercises raw method and parameter identifiers through the real runtime ABI.
    async fn r#match(&self, #[param(path)] r#type: String) -> Result<Response<String>, Error>;

    #[runtime::method(
        method = "GET",
        path = "/renamed/metadata",
        consumes = "application/vnd.fusen.request+json",
        produces = "application/vnd.fusen.response+json"
    )]
    /// Exercises all non-body HTTP metadata sources through the real runtime ABI.
    async fn metadata(
        &self,
        #[param(header, name = "x-tenant-id")] tenant: String,
        #[param(cookie, name = "session-id")] session: String,
        #[sensitive(opaque)]
        #[param(query_map)]
        query: std::collections::BTreeMap<String, String>,
        #[sensitive(opaque)]
        #[param(header_map)]
        headers: std::collections::BTreeMap<String, String>,
    ) -> Result<Response<String>, Error>;

    #[runtime::method(method = "DELETE", path = "/renamed/{id}")]
    /// Exercises an explicit synthesized body field on a replayable DELETE operation.
    async fn remove(
        &self,
        #[param(path)] id: String,
        #[param(body_field, name = "reason_code")] reason: String,
    ) -> Result<Response<String>, Error>;

    #[runtime::method(method = "DELETE", path = "/renamed/default/{id}")]
    /// Exercises the default query mapping for DELETE parameters.
    async fn remove_default(
        &self,
        #[param(path)] id: String,
        reason: String,
    ) -> Result<Response<String>, Error>;

    #[runtime::method(method = "DELETE", path = "/renamed/raw-delete/{id}")]
    /// Exercises an explicit complete JSON body on DELETE.
    async fn remove_with_body(
        &self,
        #[param(path)] id: String,
        #[param(body)] reason: String,
    ) -> Result<Response<String>, Error>;
}

/// Minimal direct implementation used by the renamed-runtime consumer test.
pub struct Handler;

impl RenamedRuntimeApi for Handler {
    async fn lookup(
        &self,
        _call: runtime::Call,
        id: String,
        _expand: Option<bool>,
    ) -> Result<Response<String>, Error> {
        Ok(Response::new(id))
    }

    async fn bindings(
        &self,
        arguments: String,
        handler: String,
        invocation: String,
        method_id: String,
        response: String,
    ) -> Result<runtime::Response<String>, runtime::Error> {
        Ok(runtime::Response::new(format!(
            "{arguments}:{handler}:{invocation}:{method_id}:{response}"
        )))
    }

    async fn r#match(&self, r#type: String) -> Result<Response<String>, Error> {
        Ok(Response::new(r#type))
    }

    async fn metadata(
        &self,
        tenant: String,
        session: String,
        query: std::collections::BTreeMap<String, String>,
        headers: std::collections::BTreeMap<String, String>,
    ) -> Result<Response<String>, Error> {
        Ok(Response::new(format!(
            "{tenant}:{session}:{}:{}",
            query.len(),
            headers.len()
        )))
    }

    async fn remove(&self, id: String, reason: String) -> Result<Response<String>, Error> {
        Ok(Response::new(format!("{id}:{reason}")))
    }

    async fn remove_default(&self, id: String, reason: String) -> Result<Response<String>, Error> {
        Ok(Response::new(format!("{id}:{reason}")))
    }

    async fn remove_with_body(
        &self,
        id: String,
        reason: String,
    ) -> Result<Response<String>, Error> {
        Ok(Response::new(format!("{id}:{reason}")))
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
        assert_eq!(descriptor.methods()[0].invocation_name(), "lookup");
        let operation = descriptor.methods()[0].http_operation();
        assert_eq!(operation.method().as_str(), "GET");
        assert_eq!(operation.path(), "/renamed/{id}");
        assert_eq!(descriptor.methods()[2].invocation_name(), "match");
        assert_eq!(
            descriptor.methods()[2].http_operation().path(),
            "/renamed/raw/{type}"
        );
        let metadata = descriptor.methods()[3].http_operation();
        assert_eq!(metadata.consumes(), "application/vnd.fusen.request+json");
        assert_eq!(metadata.produces(), "application/vnd.fusen.response+json");
        assert_eq!(metadata.parameters().len(), 4);

        let remove = descriptor.methods()[4].http_operation();
        assert_eq!(remove.method().as_str(), "DELETE");
        assert_eq!(remove.parameters()[1].name(), "reason_code");
        assert_eq!(
            remove.parameters()[1].source(),
            runtime::HttpParameterSource::BodyField
        );

        let remove_default = descriptor.methods()[5].http_operation();
        assert_eq!(remove_default.method().as_str(), "DELETE");
        assert_eq!(
            remove_default.parameters()[1].source(),
            runtime::HttpParameterSource::Query
        );

        let remove_with_body = descriptor.methods()[6].http_operation();
        assert_eq!(remove_with_body.method().as_str(), "DELETE");
        assert_eq!(
            remove_with_body.parameters()[1].source(),
            runtime::HttpParameterSource::Body
        );

        let _server = RenamedRuntimeApiServer::new(Handler);
    }
}
