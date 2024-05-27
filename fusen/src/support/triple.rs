use prost::Message;

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
    pub fn from(strs: &Vec<String>) -> Self {
        let mut trip = TripleRequestWrapper {
            serialize_type: "fastjson".to_string(),
            args: Default::default(),
            arg_types: Default::default(),
        };
        trip.args = vec![];
        for str in strs {
            trip.args.push(str.as_bytes().to_vec());
        }
        trip
    }
    pub fn get_req(self) -> Vec<String> {
        let mut res = vec![];
        for str in self.args {
            res.push(String::from_utf8(str).unwrap());
        }
        res
    }
}

impl TripleResponseWrapper {
    pub fn form(strs: &[u8]) -> Self {
        let mut trip = TripleResponseWrapper {
            serialize_type: "fastjson".to_string(),
            data: Default::default(),
            r#type: Default::default(),
        };
        trip.data = strs.to_vec();
        trip
    }
    pub fn is_empty_body(&self) -> bool {
        self.data.starts_with("null".as_bytes())
    }
}

impl TripleExceptionWrapper {
    pub fn get_buf(strs: String) -> Vec<u8> {
        let mut trip = TripleExceptionWrapper {
            language: Default::default(),
            serialization: "fastjson".to_string(),
            class_name: Default::default(),
            data: Default::default(),
        };
        trip.data = strs.as_bytes().to_vec();
        get_buf(trip.encode_to_vec())
    }
    pub fn get_err_info(&self) -> String {
        serde_json::from_slice(&self.data[..]).unwrap_or("rpc error".to_owned())
    }
}

pub fn get_buf(mut data: Vec<u8>) -> Vec<u8> {
    let mut len = data.len();
    let mut u8_array = vec![0_u8; 5];
    for idx in (1..5).rev() {
        u8_array[idx] = len as u8;
        len >>= 8;
    }
    u8_array.append(&mut data);
    u8_array
}
