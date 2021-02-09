use std::io::Error as IoError;

use mqtt_codec::error::{EncodeError, DecodeError};

#[derive(Debug)]
pub enum MQTTClientError {
  NotAuthorized,
  ConnectionLost,
  MPSCError,
  TimeoutError,
  SomethingWentWrong,
  IoError(IoError),
  EncodeError(EncodeError),
  DecodeError(DecodeError)
}  

impl From<IoError> for MQTTClientError {
  fn from(error: IoError) -> Self {
    Self::IoError(error)
  }
}

impl From<EncodeError> for MQTTClientError {
  fn from(error: EncodeError) -> Self {
    Self::EncodeError(error)
  }
}

impl From<DecodeError> for MQTTClientError {
  fn from(error: DecodeError) -> Self {
    Self::DecodeError(error)
  }
}
