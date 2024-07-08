use serde::Deserialize;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Cannot determine group GOUID: {0}")]
    UnknownGroupGUID(&'static str),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Bitly(#[from] ErrorResponse),
}

#[derive(Debug, Deserialize, thiserror::Error)]
#[error(
    "Bitly request failed with {message} ({}): {} | {:?}",
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
