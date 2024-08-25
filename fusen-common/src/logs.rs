use chrono::Local;

pub fn init_log() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_line_number(true)
        .with_thread_ids(true)
        .init();
}

pub fn get_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn get_trade_id() -> String {
    format!(
        "{}-{}",
        uuid::Uuid::new_v4(),
        Local::now().format("%Y%m%d%H%M%S")
    )
}
