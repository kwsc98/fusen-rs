pub enum CodecType {
    JSON,
    GRPC,
}

pub fn json_field_compatible(ty: &str, mut field: String) -> String {
    if ty == "String" && !field.starts_with("\"") {
        field.insert(0, '\"');
        field.insert(field.len(), '\"');
    }
    field
}
