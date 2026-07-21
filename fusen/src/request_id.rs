pub(crate) fn new_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
