use crate::error::MQTTClientError;

pub type Result<T> = std::result::Result<T, MQTTClientError>;
pub type Error = MQTTClientError;