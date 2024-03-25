use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub enum CodecType {
    JSON,
    GRPC,
}
