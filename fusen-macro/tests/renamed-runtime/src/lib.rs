use rpc::fusen_procedural_macro::fusen_trait;

#[fusen_trait]
#[rpc::fusen_procedural_macro::asset(path = "/renamed", method = get)]
pub trait RenamedRuntimeService {
    async fn lookup(&self, id: String) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpc::{
        error::FusenError,
        filter::ProceedingJoinPoint,
        fusen_procedural_macro::{fusen_service, handler},
        handler::{HandlerLoad, aspect::Aspect as AspectAlias, loadbalance::LoadBalance},
        protocol::fusen::context::FusenContext,
    };
    use std::{marker::PhantomData, sync::Arc};

    #[fusen_trait]
    #[rpc::fusen_procedural_macro::asset(path = "/generic", method = "post")]
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

    struct GenericAspect<T>(PhantomData<T>);

    #[handler(kind = Aspect)]
    impl<T> AspectAlias for GenericAspect<T>
    where
        T: Send + Sync + 'static,
    {
        async fn around(
            &self,
            join_point: ProceedingJoinPoint,
        ) -> Result<FusenContext, FusenError> {
            join_point.proceed().await
        }
    }

    struct GenericLoadBalance<T>(PhantomData<T>);

    #[handler(kind = "LoadBalance")]
    impl<T> LoadBalance for GenericLoadBalance<T>
    where
        T: Send + Sync + 'static,
    {
        async fn select<'a>(
            &'a self,
            _context: &'a FusenContext,
            invokers: Arc<Vec<Arc<rpc::fusen_internal_common::resource::service::ServiceResource>>>,
        ) -> Result<
            Option<Arc<rpc::fusen_internal_common::resource::service::ServiceResource>>,
            FusenError,
        > {
            Ok(invokers.first().cloned())
        }
    }

    #[test]
    fn renamed_runtime_and_qualified_asset_preserve_metadata() {
        let info = RenamedRuntimeServiceClient::get_service_info();
        assert_eq!(info.method_infos[0].path, "/renamed/lookup");
        assert_eq!(info.method_infos[0].method, rpc::http::Method::GET);
    }

    #[test]
    fn generic_service_and_handlers_load() {
        let _service = GenericServiceImpl::<u8>(PhantomData);
        let _aspect = GenericAspect::<u8>(PhantomData).load();
        let _load_balance = GenericLoadBalance::<u8>(PhantomData).load();
        let info = GenericServiceClient::get_service_info();
        assert_eq!(info.method_infos[0].path, "/generic/echo");
        assert_eq!(info.method_infos[0].method, rpc::http::Method::POST);
    }
}
