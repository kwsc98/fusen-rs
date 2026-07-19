use chrono::Local;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn get_now_date_time_as_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn get_now_date_time() -> String {
    Local::now().format("%Y%m%d%H%M%S").to_string()
}
