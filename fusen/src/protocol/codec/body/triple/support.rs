#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TripleRequestWrapper {
    /// hessian4
    /// json
    #[prost(string, tag = "1")]
    pub serialize_type: ::prost::alloc::string::String,
    #[prost(bytes = "vec", repeated, tag = "2")]
    pub args: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
    #[prost(string, repeated, tag = "3")]
    pub arg_types: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TripleResponseWrapper {
    #[prost(string, tag = "1")]
    pub serialize_type: ::prost::alloc::string::String,
    #[prost(bytes = "vec", tag = "2")]
    pub data: ::prost::alloc::vec::Vec<u8>,
    #[prost(string, tag = "3")]
    pub r#type: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TripleExceptionWrapper {
    #[prost(string, tag = "1")]
    pub language: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub serialization: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub class_name: ::prost::alloc::string::String,
    #[prost(bytes = "vec", tag = "4")]
    pub data: ::prost::alloc::vec::Vec<u8>,
}

impl TripleRequestWrapper {
    pub fn encode(values: Vec<Vec<u8>>) -> Self {
        let mut trip = TripleRequestWrapper {
            serialize_type: "fastjson".to_string(),
            args: Default::default(),
            arg_types: Default::default(),
        };
        trip.args = values;
        trip
    }
    pub fn decode(self) -> Vec<Vec<u8>> {
        self.args
    }
}

impl TripleResponseWrapper {
    pub fn encode(data: Vec<u8>) -> Self {
        let mut trip = TripleResponseWrapper {
            serialize_type: "fastjson".to_string(),
            data: Default::default(),
            r#type: Default::default(),
        };
        trip.data = data;
        trip
    }
    pub fn decode(self) -> Vec<u8> {
        self.data
    }
}
