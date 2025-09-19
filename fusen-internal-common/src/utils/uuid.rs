use crate::utils::uuid;
use ::uuid::Uuid;

pub fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
