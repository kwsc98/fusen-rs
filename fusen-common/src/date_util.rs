use std::time::{SystemTime, UNIX_EPOCH};

pub fn get_now_date_time_as_millis() -> i128 {
    let start = SystemTime::now();
    let since_the_epoch = start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    since_the_epoch.as_millis() as i128
}
