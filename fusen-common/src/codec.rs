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

pub fn json_field_compatible(ty: &str, mut field: String) -> String {
    if ty == "String" && !field.starts_with("\"") {
        field.insert(0, '\"');
        field.insert(field.len(), '\"');
    }
    field
}
