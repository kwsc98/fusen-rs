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
        pub enum ArgumentSource {
            Path,
            Query,
            Header,
            Cookie,
            BodyField,
            Body,
            QueryMap,
            HeaderMap,
        }

        #[derive(Clone, Copy)]
        pub struct ArgumentField;

        impl ArgumentField {
            pub const fn new(
                _name: &'static str,
                _source: ArgumentSource,
                _repeated: bool,
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

        pub struct HttpOperation;

        pub fn http_method(
            _method: http::Method,
            _path: &str,
            _consumes: &str,
            _produces: &str,
            _fields: &[ArgumentField],
        ) -> Result<HttpOperation, String> {
            Ok(HttpOperation)
        }

        pub struct MethodDescriptor;

        impl MethodDescriptor {
            pub fn new(
                _id: MethodId,
                _identity: impl Into<String>,
                _http: HttpOperation,
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
            pub const fn new(_name: &'static str, _resolver: fn() -> SensitiveShape) -> Self {
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

        pub struct Call;

        impl Call {
            pub fn new() -> Self {
                Self
            }
        }

        pub struct Arguments;

        impl Arguments {
            pub fn new() -> Self {
                Self
            }

            pub fn insert(&mut self, _name: String, _value: ()) {}
        }

        pub fn encode_argument<T>(_value: &T) -> Result<(), Error> {
            Ok(())
        }

        pub struct Response<T>(T);

        impl<T> Response<T> {
            pub fn new(body: T) -> Self {
                Self(body)
            }
        }

        #[derive(Debug)]
        pub struct Error;

        #[derive(Clone)]
        pub struct ServiceClient;

        impl ServiceClient {
            pub async fn invoke<T, F>(
                &self,
                _method: MethodId,
                _call: Call,
                _encode: F,
            ) -> Result<Response<T>, Error>
            where
                F: FnOnce() -> Result<Arguments, Error>,
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

        pub trait Interceptor: Send + Sync + 'static {}

        pub type InterceptorResult = Result<Response<Vec<u8>>, Error>;
        pub type InterceptorFuture<'a> = Pin<Box<dyn Future<Output = InterceptorResult> + Send + 'a>>;

        pub struct ServerInvocation;

        impl ServerInvocation {
            pub fn method_id(&self) -> MethodId {
                MethodId::new(0)
            }

            pub fn call(&self) -> Call {
                Call
            }

            pub fn decode_argument<T>(
                &mut self,
                _name: &str,
                _text_encoded: bool,
            ) -> Result<T, Error> {
                unimplemented!()
            }

            pub fn finish_arguments(&self) -> Result<(), Error> {
                Ok(())
            }

            pub fn encode_response<T>(self, _response: Response<T>) -> InterceptorResult {
                unimplemented!()
            }
        }

        pub fn method_not_found(_method: MethodId) -> Error {
            Error
        }

        type Dispatch<T> = for<'a> fn(&'a T, ServerInvocation) -> InterceptorFuture<'a>;

        pub struct ServerService<T>(T);

        impl<T> ServerService<T> {
            pub fn new(
                handler: T,
                _descriptor: fn() -> Result<&'static ServiceDescriptor, String>,
                _dispatch: Dispatch<T>,
            ) -> Self {
                Self(handler)
            }

            pub fn head_interceptor(self, _interceptor: impl Interceptor) -> Self {
                self
            }

            pub fn interceptor(self, _interceptor: impl Interceptor) -> Self {
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

pub use __macro::v1::{Call, Error, Response};

struct User(String);

impl __macro::v1::SensitiveFields for User {
    fn sensitive_shape() -> __macro::v1::SensitiveShape {
        __macro::v1::SensitiveShape::Opaque
    }
}

#[interface(name = "user", group = "prod", version = "1")]
trait UserApi {
    #[fusen_procedural_macro::method(method = "GET", path = "/users/{user_id}")]
    async fn get(
        &self,
        #[param(context)] call: Call,
        #[sensitive(kind = "identifier")]
        #[param(path, name = "user_id")]
        id: String,
        #[param(query)] expand: Option<bool>,
    ) -> Result<Response<User>, Error>;

    #[fusen_procedural_macro::method(method = "POST", path = "/users/batch")]
    async fn batch(
        &self,
        #[sensitive(opaque)] names: Vec<String>,
        notify: bool,
    ) -> Result<Response<User>, Error>;

    #[fusen_procedural_macro::method(method = "POST", path = "/users/bindings")]
    async fn bindings(
        &self,
        arguments: String,
        handler: String,
        invocation: String,
        method_id: String,
        response: String,
    ) -> Result<Response<User>, Error>;

    #[fusen_procedural_macro::method(method = "GET", path = "/users/labels")]
    async fn labels(
        &self,
        #[param(query, repeated)] labels: Vec<String>,
    ) -> Result<Response<User>, Error>;

    #[fusen_procedural_macro::method(
        method = "GET",
        path = "/users/metadata",
        consumes = "application/vnd.fusen.request+json",
        produces = "application/vnd.fusen.response+json"
    )]
    async fn metadata(
        &self,
        #[param(header, name = "x-tenant-id")] tenant: String,
        #[param(cookie, name = "session-id")] session: String,
        #[param(query_map)] query: String,
        #[param(header_map)] headers: String,
    ) -> Result<Response<User>, Error>;

    #[fusen_procedural_macro::method(method = "GET", path = "/users/raw/{type}")]
    async fn r#match(
        &self,
        #[param(path)] r#type: String,
    ) -> Result<Response<User>, Error>;
}

struct Handler;

impl UserApi for Handler {
    async fn get(
        &self,
        _call: Call,
        id: String,
        _expand: Option<bool>,
    ) -> Result<Response<User>, Error> {
        Ok(Response::new(User(id)))
    }

    async fn batch(&self, names: Vec<String>, notify: bool) -> Result<Response<User>, Error> {
        Ok(Response::new(User(format!(
            "{}:{notify}",
            names.join(",")
        ))))
    }

    async fn bindings(
        &self,
        arguments: String,
        handler: String,
        invocation: String,
        method_id: String,
        response: String,
    ) -> Result<Response<User>, Error> {
        Ok(Response::new(User(format!(
            "{arguments}:{handler}:{invocation}:{method_id}:{response}"
        ))))
    }

    async fn labels(&self, labels: Vec<String>) -> Result<Response<User>, Error> {
        Ok(Response::new(User(labels.join(","))))
    }

    async fn metadata(
        &self,
        tenant: String,
        session: String,
        query: String,
        headers: String,
    ) -> Result<Response<User>, Error> {
        Ok(Response::new(User(format!(
            "{tenant}:{session}:{query}:{headers}"
        ))))
    }

    async fn r#match(&self, r#type: String) -> Result<Response<User>, Error> {
        Ok(Response::new(User(r#type)))
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
