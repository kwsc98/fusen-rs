use uuid::uuid;

pub mod date_util;


pub fn get_uuid() -> String {
    uuid!("550e8400e29b41d4a716446655440000").to_string()
}
