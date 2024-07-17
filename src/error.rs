use serde::Deserialize;

use crate::config::APP;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{APP}: operation '{0}' could not complete under offline mode")]
    Offline(&'static str),

    #[error("{APP}: cannot determine group GOUID: {0}")]
    UnknownGroupGUID(&'static str),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Bitly(#[from] ErrorResponse),
}

#[derive(Debug, Deserialize, thiserror::Error)]
#[error(
    "{APP}: Bitly request failed with {message} ({}): {} | {:?}",
    resource.as_deref().unwrap_or("?"),
    description.as_deref().unwrap_or("?"),
    errors.as_deref().unwrap_or_default(),
)]
pub struct ErrorResponse {
    message: String,
    description: Option<String>,
    resource: Option<String>,
    #[serde(skip_deserializing)]
    errors: Option<Vec<FieldError>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct FieldError {
    field: String,
    error_code: String,
    message: String,
}
