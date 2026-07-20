use crate::{error::FusenError, protocol::fusen::context::FusenContext};
use fusen_internal_common::{BoxFutureV2, resource::service::ServiceResource};
use rand::Rng;
use std::sync::Arc;

#[allow(async_fn_in_trait)]
pub trait LoadBalance {
    async fn select<'a>(
        &'a self,
        context: &'a FusenContext,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> Result<Option<Arc<ServiceResource>>, FusenError>;
}

pub trait LoadBalanceDyn: Send + Sync {
    fn select_dyn<'a>(
        &'a self,
        context: &'a FusenContext,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> BoxFutureV2<'a, Result<Option<Arc<ServiceResource>>, FusenError>>;
}

pub struct DefaultLoadBalance;

impl LoadBalanceDyn for DefaultLoadBalance {
    fn select_dyn(
        &'_ self,
        _context: &'_ FusenContext,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> BoxFutureV2<'_, Result<Option<Arc<ServiceResource>>, FusenError>> {
        Box::pin(async move {
            if invokers.is_empty() {
                return Ok(None);
            }
            let max_weight = invokers
                .iter()
                .map(|resource| resource.weight.unwrap_or(1.0))
                .filter(|weight| weight.is_finite() && *weight > 0.0)
                .fold(0.0_f64, f64::max);
            if max_weight <= 0.0 {
                return Ok(None);
            }
            let total = invokers
                .iter()
                .map(|resource| resource.weight.unwrap_or(1.0))
                .filter(|weight| weight.is_finite() && *weight > 0.0)
                .map(|weight| weight / max_weight)
                .sum::<f64>();
            let mut target = rand::rng().random_range(0.0..total);
            let mut last_valid = None;
            for resource in invokers.iter() {
                let weight = resource.weight.unwrap_or(1.0);
                if !weight.is_finite() || weight <= 0.0 {
                    continue;
                }
                last_valid = Some(resource.clone());
                let weight = weight / max_weight;
                if target < weight {
                    return Ok(Some(resource.clone()));
                }
                target -= weight;
            }
            Ok(last_valid)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::fusen::{
        context::FusenContext,
        request::{FusenRequest, Path},
        service::{MethodInfo, ServiceDesc},
    };
    use fusen_internal_common::protocol::WireProtocol;
    use http::Method;

    fn resource(address: &str, weight: f64) -> Arc<ServiceResource> {
        Arc::new(ServiceResource {
            service_id: address.into(),
            group: None,
            version: None,
            methods: Vec::new(),
            addr: address.into(),
            weight: Some(weight),
            metadata: Default::default(),
        })
    }

    fn context() -> FusenContext {
        let service = ServiceDesc::new("demo", None, None);
        FusenContext {
            unique_identifier: "request".into(),
            metadata: Default::default(),
            method_info: Arc::new(MethodInfo::new(
                service,
                "call".into(),
                Method::GET,
                "/demo".into(),
                Vec::new(),
            )),
            request: FusenRequest {
                protocol: WireProtocol::Fusen,
                path: Path {
                    method: Method::GET,
                    path: "/demo".into(),
                },
                endpoint: None,
                path_parameters: Default::default(),
                query_parameters: Default::default(),
                headers: Default::default(),
                body: None,
            },
            response: None,
        }
    }

    #[tokio::test]
    async fn huge_weights_do_not_overflow_selection() {
        let resources = Arc::new(vec![
            resource("http://one", f64::MAX),
            resource("http://two", f64::MAX),
            resource("http://invalid", f64::INFINITY),
        ]);
        let selected = DefaultLoadBalance
            .select_dyn(&context(), resources)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            selected.service_id.as_str(),
            "http://one" | "http://two"
        ));
    }
}
