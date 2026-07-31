extern crate self as fusen_rs;

use fusen_procedural_macro::interface;

pub mod __macro {
    pub mod v1 {
        use std::{future::Future, marker::PhantomData, pin::Pin};

        #[derive(Clone, Copy)]
        pub struct MethodId(u16);

        impl MethodId {
            pub const fn new(value: u16) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u16 {
                self.0
            }
        }

        #[derive(Clone, Copy)]
        pub enum RpcFieldSource {
            Path,
            Query,
            BodyField,
            Body,
        }

        #[derive(Clone, Copy)]
        pub struct RpcField;

        impl RpcField {
            pub const fn new(
                _name: &'static str,
                _source: RpcFieldSource,
                _repeated: bool,
                _parse_spring_json_primitive: bool,
            ) -> Self {
                Self
            }
        }

        pub mod http {
            #[derive(Clone, Copy)]
            pub struct Method;

            impl Method {
                pub const GET: Self = Self;
                pub const POST: Self = Self;
                pub const PUT: Self = Self;
                pub const PATCH: Self = Self;
                pub const DELETE: Self = Self;
                pub const HEAD: Self = Self;
                pub const OPTIONS: Self = Self;
            }
        }

        #[derive(Debug)]
        pub struct DescriptorError;

        impl std::fmt::Display for DescriptorError {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("invalid descriptor")
            }
        }

        pub struct SpringCloudMethod;

        pub fn http_method(
            _method: http::Method,
            _path: &str,
            _fields: &[RpcField],
        ) -> Result<SpringCloudMethod, String> {
            Ok(SpringCloudMethod)
        }

        pub struct MethodDescriptor;

        impl MethodDescriptor {
            pub fn new(
                _id: MethodId,
                _identity: impl Into<String>,
                _spring: Option<SpringCloudMethod>,
            ) -> Result<Self, DescriptorError> {
                Ok(Self)
            }

            pub fn with_sensitivity(self, _sensitivity: MethodSensitivity) -> Self {
                self
            }
        }

        #[derive(Clone, Copy)]
        pub struct SensitivityKind;

        impl SensitivityKind {
            pub fn new(_kind: &'static str) -> Result<Self, DescriptorError> {
                Ok(Self)
            }
        }

        pub enum SensitiveShape {
            Opaque,
            Kind(SensitivityKind),
        }

        pub trait SensitiveFields {
            fn sensitive_shape() -> SensitiveShape;
        }

        macro_rules! opaque_sensitive_fields {
            ($($type:ty),+ $(,)?) => {
                $(
                    impl SensitiveFields for $type {
                        fn sensitive_shape() -> SensitiveShape {
                            SensitiveShape::Opaque
                        }
                    }
                )+
            };
        }

        opaque_sensitive_fields!((), bool, String);

        impl<T: SensitiveFields> SensitiveFields for Option<T> {
            fn sensitive_shape() -> SensitiveShape {
                T::sensitive_shape()
            }
        }

        impl<T: SensitiveFields> SensitiveFields for Vec<T> {
            fn sensitive_shape() -> SensitiveShape {
                T::sensitive_shape()
            }
        }

        pub struct SensitiveArgument;

        impl SensitiveArgument {
            pub const fn new(
                _name: &'static str,
                _resolver: fn() -> SensitiveShape,
            ) -> Self {
                Self
            }
        }

        pub struct MethodSensitivity;

        impl MethodSensitivity {
            pub fn new(
                _arguments: Vec<SensitiveArgument>,
                _response: Option<fn() -> SensitiveShape>,
            ) -> Self {
                Self
            }
        }

        pub struct ServiceSelector;

        impl ServiceSelector {
            pub fn new(
                _name: impl Into<String>,
                _group: Option<String>,
                _version: Option<String>,
            ) -> Result<Self, DescriptorError> {
                Ok(Self)
            }
        }

        pub struct ServiceDescriptor;

        impl ServiceDescriptor {
            pub fn new(
                _selector: ServiceSelector,
                _methods: Vec<MethodDescriptor>,
            ) -> Result<Self, DescriptorError> {
                Ok(Self)
            }
        }

        pub struct RpcCall;

        impl RpcCall {
            pub fn new() -> Self {
                Self
            }
        }

        pub struct RpcArguments;

        impl RpcArguments {
            pub fn new() -> Self {
                Self
            }

            pub fn insert(&mut self, _name: String, _value: ()) {}
        }

        pub fn encode_argument<T>(_value: &T) -> Result<(), RpcError> {
            Ok(())
        }

        pub struct RpcResponse<T>(T);

        impl<T> RpcResponse<T> {
            pub fn new(body: T) -> Self {
                Self(body)
            }
        }

        #[derive(Debug)]
        pub struct RpcError;

        #[derive(Clone)]
        pub struct ServiceClient;

        impl ServiceClient {
            pub async fn invoke<T, F>(
                &self,
                _method: MethodId,
                _call: RpcCall,
                _encode: F,
            ) -> Result<RpcResponse<T>, RpcError>
            where
                F: FnOnce() -> Result<RpcArguments, RpcError>,
            {
                unimplemented!()
            }
        }

        pub struct ClientRuntime;

        pub struct ClientBuilder<C>(PhantomData<C>);

        impl<C> ClientBuilder<C> {
            pub fn new(
                _runtime: &ClientRuntime,
                _descriptor: fn() -> Result<&'static ServiceDescriptor, String>,
                _create: fn(ServiceClient) -> C,
            ) -> Self {
                Self(PhantomData)
            }
        }

        pub trait Middleware: Send + Sync + 'static {}

        pub type MiddlewareResult = Result<RpcResponse<Vec<u8>>, RpcError>;
        pub type MiddlewareFuture<'a> =
            Pin<Box<dyn Future<Output = MiddlewareResult> + Send + 'a>>;

        pub struct ServerInvocation;

        impl ServerInvocation {
            pub fn method_id(&self) -> MethodId {
                MethodId::new(0)
            }

            pub fn rpc_call(&self) -> RpcCall {
                RpcCall
            }

            pub fn decode_argument<T>(
                &mut self,
                _name: &str,
                _parse_primitive: bool,
            ) -> Result<T, RpcError> {
                unimplemented!()
            }

            pub fn finish_arguments(&self) -> Result<(), RpcError> {
                Ok(())
            }

            pub fn encode_response<T>(self, _response: RpcResponse<T>) -> MiddlewareResult {
                unimplemented!()
            }
        }

        pub fn method_not_found(_method: MethodId) -> RpcError {
            RpcError
        }

        type Dispatch<T> =
            for<'a> fn(&'a T, ServerInvocation) -> MiddlewareFuture<'a>;

        pub struct ServerService<T>(T);

        impl<T> ServerService<T> {
            pub fn new(
                handler: T,
                _descriptor: fn() -> Result<&'static ServiceDescriptor, String>,
                _dispatch: Dispatch<T>,
            ) -> Self {
                Self(handler)
            }

            pub fn head_middleware(self, _middleware: impl Middleware) -> Self {
                self
            }

            pub fn middleware(self, _middleware: impl Middleware) -> Self {
                self
            }

            pub fn into_prepared(self) -> PreparedService {
                PreparedService
            }
        }

        pub struct PreparedService;

        pub trait IntoServerService {
            fn into_server_service(self) -> PreparedService;
        }
    }
}

pub use __macro::v1::{RpcCall, RpcError, RpcResponse};

struct User(String);

impl __macro::v1::SensitiveFields for User {
    fn sensitive_shape() -> __macro::v1::SensitiveShape {
        __macro::v1::SensitiveShape::Opaque
    }
}

#[interface(name = "user", group = "prod", version = "1")]
trait UserApi {
    #[fusen_procedural_macro::method(
        method = "GET", path = "/users/{user_id}"
    )]
    async fn get(
        &self,
        #[param(context)] call: RpcCall,
        #[sensitive(kind = "identifier")]
        #[param(path, name = "user_id")] id: String,
        #[param(query)] expand: Option<bool>,
    ) -> Result<RpcResponse<User>, RpcError>;

    #[fusen_procedural_macro::method(
        method = "POST", path = "/users/batch"
    )]
    async fn batch(
        &self,
        #[sensitive(opaque)]
        names: Vec<String>,
        notify: bool,
    ) -> Result<RpcResponse<User>, RpcError>;
}

struct Handler;

impl UserApi for Handler {
    async fn get(
        &self,
        _call: RpcCall,
        id: String,
        _expand: Option<bool>,
    ) -> Result<RpcResponse<User>, RpcError> {
        Ok(RpcResponse::new(User(id)))
    }

    async fn batch(
        &self,
        names: Vec<String>,
        notify: bool,
    ) -> Result<RpcResponse<User>, RpcError> {
        Ok(RpcResponse::new(User(format!(
            "{}:{notify}",
            names.join(",")
        ))))
    }
}

fn assert_interface<T: UserApi>() {}

fn main() {
    assert_interface::<UserApiClient>();
    assert_interface::<Handler>();
    let runtime = __macro::v1::ClientRuntime;
    let _: __macro::v1::ClientBuilder<UserApiClient> = UserApiClient::builder(&runtime);
    let _server = UserApiServer::new(Handler);
    let _ = UserApiClient::descriptor();
}
