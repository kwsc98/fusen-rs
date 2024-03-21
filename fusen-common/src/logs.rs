use tracing_subscriber::fmt::writer::MakeWriterExt;

pub fn init_log() {
    let stdout = std::io::stdout.with_max_level(tracing::Level::DEBUG);
    tracing_subscriber::fmt()
        .with_writer(stdout)
        .with_line_number(true)
        .with_thread_ids(true)
        .init();
}

pub fn get_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
