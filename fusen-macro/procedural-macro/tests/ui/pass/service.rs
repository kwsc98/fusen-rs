extern crate self as fusen_rs;

use fusen_procedural_macro::service;

pub mod __macro {
    use std::future::Future;
    use std::pin::Pin;

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

    pub enum Idempotency {
        None,
        Idempotent,
        Safe,
    }

    pub enum SpringCloudParameterSource {
        Path,
        Query,
        Body,
    }

    pub enum SpringCloudParameterCardinality {
        Scalar,
        Repeated,
    }

    pub struct SpringCloudParameter;

    impl SpringCloudParameter {
        pub fn new(
            _name: impl Into<String>,
            _source: SpringCloudParameterSource,
            _cardinality: SpringCloudParameterCardinality,
        ) -> Result<Self, ()> {
            Ok(Self)
        }
    }

    pub mod http {
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

    pub struct SpringCloudMethod;

    impl SpringCloudMethod {
        pub fn new(
            _method: http::Method,
            _path: impl Into<String>,
            _parameters: Vec<SpringCloudParameter>,
        ) -> Result<Self, ()> {
            Ok(Self)
        }
    }

    pub struct MethodDescriptor;

    impl MethodDescriptor {
        pub fn new(
            _id: MethodId,
            _identity: impl Into<String>,
            _idempotency: Idempotency,
            _spring: Option<SpringCloudMethod>,
        ) -> Result<Self, ()> {
            Ok(Self)
        }
    }

    pub struct ServiceSelector;

    impl ServiceSelector {
        pub fn new(
            _name: impl Into<String>,
            _group: Option<String>,
            _version: Option<String>,
        ) -> Result<Self, ()> {
            Ok(Self)
        }
    }

    pub struct ServiceDescriptor;

    impl ServiceDescriptor {
        pub fn new(
            _selector: ServiceSelector,
            _methods: Vec<MethodDescriptor>,
        ) -> Result<Self, ()> {
            Ok(Self)
        }
    }

    #[derive(Clone)]
    pub struct ServiceClient;

    impl ServiceClient {
        pub async fn invoke<T, F>(
            &self,
            _method: MethodId,
            build_arguments: F,
        ) -> Result<T, RpcError>
        where
            F: FnOnce() -> Result<Arguments, RpcError> + Send,
        {
            let _arguments = build_arguments()?;
            unimplemented!()
        }
    }

    pub struct ClientRuntime;
    pub struct ClientError;
    pub struct RpcError;
    pub struct Value;
    pub struct Arguments;

    impl Arguments {
        pub fn new() -> Self {
            Self
        }

        pub fn insert(&mut self, _name: String, _value: Value) {}
    }

    pub fn encode_argument<T: ?Sized>(_value: &T) -> Result<Value, RpcError> {
        Ok(Value)
    }

    pub fn decode_argument<T>(
        _arguments: &mut Arguments,
        _name: &str,
    ) -> Result<T, RpcError> {
        unimplemented!()
    }

    pub fn finish_arguments(_arguments: &Arguments) -> Result<(), RpcError> {
        Ok(())
    }

    pub fn method_not_found(_method: MethodId) -> RpcError {
        RpcError
    }

    pub trait Middleware {}
    pub trait Router {}
    pub trait LoadBalancer {}

    pub enum WireProtocol {
        FusenV1,
        SpringCloudV1,
    }

    pub struct ServiceClientBuilder;

    impl ServiceClientBuilder {
        pub fn new(_runtime: &ClientRuntime, _descriptor: &'static ServiceDescriptor) -> Self {
            Self
        }

        pub fn direct(self, _endpoint: impl AsRef<str>) -> Self {
            self
        }

        pub fn discover(self) -> Self {
            self
        }

        pub fn protocol(self, _protocol: WireProtocol) -> Self {
            self
        }

        pub fn middleware(self, _middleware: impl Middleware) -> Self {
            self
        }

        pub fn router(self, _router: impl Router) -> Self {
            self
        }

        pub fn load_balancer(self, _load_balancer: impl LoadBalancer) -> Self {
            self
        }

        pub async fn connect(self) -> Result<ServiceClient, ClientError> {
            Ok(ServiceClient)
        }
    }

    pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
    pub type RpcResult = Result<ServerResponse, RpcError>;
    pub struct ServerResponse;

    pub struct ServerInvocation;

    impl ServerInvocation {
        pub fn method_id(&self) -> MethodId {
            MethodId::new(0)
        }

        pub fn take_arguments(&mut self) -> Arguments {
            Arguments::new()
        }

        pub fn encode_result<T>(self, _value: T) -> RpcResult {
            Ok(ServerResponse)
        }
    }

    pub type Dispatch<T> =
        for<'a> fn(&'a T, ServerInvocation) -> BoxFuture<'a, RpcResult>;

    pub struct ServerService<T>(T);

    impl<T> ServerService<T> {
        pub fn new(
            service: T,
            _descriptor: &'static ServiceDescriptor,
            _dispatch: Dispatch<T>,
        ) -> Self {
            Self(service)
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

struct User;
struct CreateUser;

#[service(name = "user", group = "prod", version = "1")]
trait UserService {
    #[fusen_procedural_macro::method(
        idempotency = "safe",
        spring(
            method = "GET",
            path = "/users/{id}",
            query = ["expand", "labels"]
        )
    )]
    async fn get(
        &self,
        id: String,
        expand: Option<bool>,
        labels: Vec<String>,
    ) -> Result<User, __macro::RpcError>;

    #[fusen_procedural_macro::method(
        idempotency = "none",
        spring(method = "POST", path = "/users", body = "request")
    )]
    async fn create(&self, request: CreateUser) -> Result<User, __macro::RpcError>;
}

struct UserServiceImpl;

impl UserService for UserServiceImpl {
    async fn get(
        &self,
        _id: String,
        _expand: Option<bool>,
        _labels: Vec<String>,
    ) -> Result<User, __macro::RpcError> {
        Ok(User)
    }

    async fn create(&self, _request: CreateUser) -> Result<User, __macro::RpcError> {
        Ok(User)
    }
}

fn main() {
    let runtime = __macro::ClientRuntime;
    let _builder = UserServiceClient::builder(&runtime)
        .direct("http://127.0.0.1:8080")
        .protocol(__macro::WireProtocol::FusenV1);
    let _server = UserServiceServer::new(UserServiceImpl);
    let _ = UserServiceClient::descriptor();
}
