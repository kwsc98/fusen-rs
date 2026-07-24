use rpc::fusen_trait;

#[fusen_trait]
#[rpc::asset(path = "/renamed", method = get)]
pub trait RenamedRuntimeService {
    async fn lookup(&self, id: String) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpc::{
        Middleware, Next, RpcContext, RpcResult,
        client::cluster::{InstanceSnapshot, LoadBalancer},
        error::FusenError,
        fusen_service,
    };
    use std::marker::PhantomData;

    #[fusen_trait]
    #[rpc::asset(path = "/generic", method = "post")]
    trait GenericService: Send + Sync
    where
        Self: 'static,
    {
        async fn echo(&self, value: String, pair: (u8, u8)) -> String;
    }

    struct GenericServiceImpl<T>(PhantomData<T>);

    #[fusen_service]
    impl<T> GenericService for GenericServiceImpl<T>
    where
        T: Send + Sync + 'static,
    {
        async fn echo(
            &self,
            mut value: String,
            (left, right): (u8, u8),
        ) -> Result<String, FusenError> {
            value.push_str(&format!("{left}{right}"));
            Ok(value)
        }
    }

    struct GenericMiddleware<T>(PhantomData<T>);

    impl<T> Middleware for GenericMiddleware<T>
    where
        T: Send + Sync + 'static,
    {
        async fn handle<'a>(&'a self, context: RpcContext, next: Next<'a>) -> RpcResult {
            next.run(context).await
        }
    }

    struct GenericLoadBalance<T>(PhantomData<T>);

    impl<T> LoadBalancer for GenericLoadBalance<T>
    where
        T: Send + Sync + 'static,
    {
        fn select(
            &self,
            _context: &RpcContext,
            invokers: &InstanceSnapshot,
        ) -> Result<usize, FusenError> {
            (!invokers.is_empty()).then_some(0).ok_or_else(|| {
                FusenError::ServiceUnavailable("no healthy service instances".into())
            })
        }
    }

    #[test]
    fn renamed_runtime_and_qualified_asset_preserve_metadata() {
        let info = RenamedRuntimeServiceClient::service_descriptor();
        assert_eq!(info.methods()[0].path(), "/renamed/lookup");
        assert_eq!(info.methods()[0].method(), &http::Method::GET);
    }

    #[test]
    fn generic_service_and_extensions_compile() {
        let _service = GenericServiceImpl::<u8>(PhantomData);
        let _middleware = GenericMiddleware::<u8>(PhantomData);
        let _load_balance = GenericLoadBalance::<u8>(PhantomData);
        let info = GenericServiceClient::service_descriptor();
        assert_eq!(info.methods()[0].path(), "/generic/echo");
        assert_eq!(info.methods()[0].method(), &http::Method::POST);
    }
}
