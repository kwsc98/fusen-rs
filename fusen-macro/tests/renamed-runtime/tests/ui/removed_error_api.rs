use runtime::{ClientError, Error, ErrorCategory, ErrorOrigin, ServerError};

fn removed_invocation_api(status: runtime::__macro::v1::http::StatusCode) {
    let _ = Error::new(ErrorCategory::Internal, "old_error", "removed");
    let _ = Error::application(status, "old_application", "removed");
    let _ = ErrorCategory::Application;
    let _ = ErrorOrigin::Application;
    let _ = ErrorCategory::Internal.status();
}

fn removed_lifecycle_api(client: &ClientError, server: &ServerError) {
    let _ = client.message_ref();
    let _ = server.message_ref();
}

fn main() {}
