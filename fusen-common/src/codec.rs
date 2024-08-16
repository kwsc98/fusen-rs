use bytes::{Buf, Bytes, BytesMut};
use serde::Serialize;

use crate::error::{BoxError, FusenError};

pub enum CodecType {
    JSON,
    GRPC,
}

impl From<&str> for CodecType {
    fn from(value: &str) -> Self {
        if value.to_lowercase().contains("grpc") {
            Self::GRPC
        } else {
            Self::JSON
        }
    }
}

pub fn json_field_compatible(ty: &str, field: Bytes) -> Result<String, FusenError> {
    let mut field_str =
        String::from_utf8(field.to_vec()).map_err(|e| FusenError::Info(e.to_string()))?;
    if field_str.to_lowercase().starts_with("null") {
        return Err(FusenError::Null);
    }
    if ty == "String" && !field_str.starts_with('"') {
        field_str.insert(0, '"');
        field_str.insert(field_str.len(), '"');
    }
    Ok(field_str)
}

pub fn byte_to_vec(bytes: Bytes) -> Bytes {
    let body = bytes.chunk();
    if !body.starts_with(b"[") {
        let mut mut_bytes = BytesMut::from("[\"");
        mut_bytes.extend(bytes);
        mut_bytes.extend_from_slice(b"\"]");
        return mut_bytes.into();
    }
    bytes
}

pub fn object_to_bytes<T: Serialize>(obj: &T) -> Result<Bytes, BoxError> {
    let bytes = serde_json::to_vec(obj)?;
    Ok(Bytes::copy_from_slice(&bytes))
}
