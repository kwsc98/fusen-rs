use crate::{
    error::FusenError,
    filter::{FusenFilter, ProceedingJoinPoint},
    handler::{Handler, HandlerContext, HandlerController, HandlerInfo},
    protocol::{
        codec::{FusenHttpCodec, RequestCodec, ResponseCodec},
        fusen::{
            context::FusenContext,
            request::FusenRequest,
            response::HttpStatus,
            service::{MethodInfo, ServiceInfo},
        },
    },
};
use bytes::Bytes;
use fusen_internal_common::{
    protocol::Protocol,
    resource::service::{MethodResource, ServiceResource},
    utils::uuid::uuid,
};
use fusen_register::{Register, directory::Directory};
use http_body_util::{BodyExt, combinators::BoxBody};
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use serde_json::Value;
use std::{
    collections::{HashMap, LinkedList},
    convert::Infallible,
    sync::Arc,
};

pub struct FusenClientContextBuilder {
    register: Option<Box<dyn Register>>,
    handler_context: HandlerContext,
}

impl Default for FusenClientContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl FusenClientContextBuilder {
    pub fn new() -> Self {
        Self {
            register: Default::default(),
            handler_context: HandlerContext::default(),
        }
    }

    pub fn builder(self) -> FusenClientContext {
        FusenClientContext {
            register: self.register.map(Arc::new),
            handler_context: self.handler_context,
            http_client: Box::leak(Box::new(HttpClient::default())),
        }
    }

    pub fn register(mut self, register: Box<dyn Register>) -> Self {
        let _ = self.register.insert(register);
        self
    }

    pub fn handler(mut self, handler: Handler) -> Self {
        self.handler_context.load_handler(handler);
        self
    }
}

pub struct FusenClientContext {
    register: Option<Arc<Box<dyn Register>>>,
    handler_context: HandlerContext,
    http_client: &'static dyn FusenFilter,
}

impl FusenClientContext {
    pub async fn init_client(
        &mut self,
        service_info: ServiceInfo,
        protocol: Protocol,
        handlers: Option<Vec<&str>>,
    ) -> Result<FusenClient, FusenError> {
        let mut methods = HashMap::new();
        for method_info in service_info.method_infos {
            methods.insert(method_info.method_name.to_string(), Arc::new(method_info));
        }
        if let Some(handlers) = handlers {
            self.handler_context.load_controller(HandlerInfo {
                service_desc: service_info.service_desc.clone(),
                handlers: handlers.iter().map(|e| e.to_string()).collect(),
            });
        }
        let handler_controller = self
            .handler_context
            .get_controller(&service_info.service_desc);
        let mut service_resource = ServiceResource {
            service_id: service_info.service_desc.service_id,
            group: service_info.service_desc.group,
            version: service_info.service_desc.version,
            methods: methods
                .values()
                .map(|e| MethodResource {
                    method_name: e.method_name.to_string(),
                    path: e.path.to_string(),
                    method: e.method.to_string(),
                })
                .collect(),
            addr: Default::default(),
            weight: Some(1.0),
            metadata: Default::default(),
        };
        let directory = if let Protocol::Host(host) = &protocol {
            service_resource.addr = host.to_string();
            let directory = Directory::default();
            directory
                .change(vec![service_resource])
                .await
                .map_err(|error| FusenError::Error(Box::new(error)))?;
            directory
        } else {
            let Some(register) = &self.register else {
                return Err(FusenError::ErrorMessage("not find register"));
            };
            register
                .subscribe(service_resource, protocol.clone())
                .await
                .map_err(|error| FusenError::Error(Box::new(error)))?
        };

        Ok(FusenClient {
            http_client: self.http_client,
            protocol,
            directory,
            handler_controller: handler_controller.clone(),
            methods,
        })
    }
}

pub struct FusenClient {
    pub http_client: &'static dyn FusenFilter,
    pub protocol: Protocol,
    pub directory: Directory,
    pub handler_controller: HandlerController,
    pub methods: HashMap<String, Arc<MethodInfo>>,
}

impl FusenClient {
    pub async fn invoke(
        &self,
        method_name: &str,
        method: &str,
        path: &str,
        field_pats: &[&str],
        request_bodys: LinkedList<Value>,
    ) -> Result<Value, FusenError> {
        let mut fusen_request = FusenRequest::init_request(
            self.protocol.clone(),
            method,
            path,
            field_pats,
            request_bodys,
        )?;
        let resources = self
            .directory
            .get()
            .await
            .map_err(|error| FusenError::Error(Box::new(error)))?;
        let Some(resource) = self
            .handler_controller
            .load_balance
            .select_(resources)
            .await?
        else {
            return Err(FusenError::HttpError(HttpStatus {
                status: 503,
                message: Some("Service Unavailable".to_string()),
            }));
        };
        fusen_request.addr = Some(resource.addr.to_owned());
        let method_info = self.methods.get(method_name).unwrap();
        let fusen_context = FusenContext {
            unique_identifier: uuid(),
            metadata: Default::default(),
            method_info: method_info.clone(),
            request: fusen_request,
            response: Default::default(),
        };
        let proceeding_join_point = ProceedingJoinPoint::new(
            self.handler_controller.aspect.clone(),
            self.http_client,
            fusen_context,
        );
        let context = proceeding_join_point.proceed().await?;
        let response = context.response.ok_or(FusenError::Impossible)?;
        match response.body {
            Some(value) => Ok(value),
            None => Err(FusenError::HttpError(response.http_status)),
        }
    }
}

struct HttpClient {
    pub http_codec: FusenHttpCodec,
    pub http_client: Client<HttpsConnector<HttpConnector>, BoxBody<Bytes, Infallible>>,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self {
            http_codec: FusenHttpCodec::default(),
            http_client: Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(HttpsConnector::new()),
        }
    }
}

impl FusenFilter for HttpClient {
    fn call(
        &'static self,
        join_point: ProceedingJoinPoint,
    ) -> fusen_internal_common::BoxFuture<Result<FusenContext, FusenError>> {
        Box::pin(async move {
            let mut fusen_context = join_point.context;
            let http_request = RequestCodec::encode(&self.http_codec, &mut fusen_context.request)?;
            let response = self
                .http_client
                .request(http_request)
                .await
                .map_err(|error| FusenError::Error(Box::new(error)))?;
            let fusen_response =
                ResponseCodec::decode(&self.http_codec, response.map(|e| e.boxed())).await?;
            let _ = fusen_context.response.insert(fusen_response);
            Ok(fusen_context)
        })
    }
}
