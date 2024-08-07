use serde::Deserialize;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("operation '{0}' could not complete under offline mode")]
    Offline(&'static str),

    #[error("cannot determine group GOUID: {0}")]
    UnknownGroupGUID(&'static str),

    #[error(transparent)]
    Io(#[from] std::io::Error),

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
    pub(crate) message: String,
    pub(crate) description: Option<String>,
    pub(crate) resource: Option<String>,
    #[serde(skip_deserializing)]
    pub(crate) errors: Option<Vec<FieldError>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct FieldError {
    pub(crate) field: String,
    pub(crate) error_code: String,
    pub(crate) message: String,
}
