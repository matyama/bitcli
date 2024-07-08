use std::borrow::Cow;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::Config;
use crate::error::{Error, Result};

/// API request to get user info
///
/// <https://dev.bitly.com/api-reference/#getUser>
#[derive(Debug, Deserialize, Serialize)]
pub struct User {
    pub is_active: bool,
    pub default_group_guid: String,
}

/// API request to create a bitlink
///
/// <https://dev.bitly.com/api-reference/#createBitlink>
#[derive(Serialize)]
struct Shorten<'a> {
    long_url: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    domain: Option<&'a str>,
    group_guid: Cow<'a, str>,
}

impl std::fmt::Debug for Shorten<'_> {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shorten")
            .field("long_url", &self.long_url.as_str())
            .field("domain", &self.domain)
            .field("group_guid", &self.group_guid.as_ref())
            .finish()
    }
}

#[derive(Debug, Deserialize)]
pub struct Bitlink {
    pub link: Url,
    #[allow(dead_code)]
    pub id: String,
}

impl std::fmt::Display for Bitlink {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.link)
    }
}

macro_rules! parse_response {
    ($resp:expr => $ok:ident $(| $oks:ident)* || $err:ident $(| $errs:ident)*) => {
        let resp = $resp;
        match resp.status() {
            StatusCode::$ok $(| StatusCode::$oks)* => match resp.json().await {
                Ok(resp) => Ok(resp),
                Err(err) => panic!("API violation: invalid response {err:?}"),
            },

            StatusCode::$err $(| StatusCode::$errs)* => match resp.json().await {
                Ok(resp) => Err(Error::Bitly(resp)),
                Err(err) => panic!("API violation: invalid error response {err:?}"),
            },

            code => unreachable!("API violation: unexpected status code '{code}'"),
        }
    };
}

pub struct Client {
    cfg: Config,
    http: reqwest::Client,
}

// TODO: handle timeouts, cancellation, API limits (see `GET /v4/user/platform_limits`), etc.
impl Client {
    #[inline]
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            http: reqwest::Client::new(),
        }
    }

    pub async fn fetch_user(&self) -> Result<User> {
        //println!("fetching user info");
        let resp = self
            .http
            .get("https://api-ssl.bitly.com/v4/user")
            .bearer_auth(self.cfg.api_token())
            .send()
            .await?;

        parse_response! {
            resp => OK || FORBIDDEN | NOT_FOUND | INTERNAL_SERVER_ERROR | SERVICE_UNAVAILABLE
        }
    }

    pub async fn shorten(&self, long_url: Url) -> Result<Bitlink> {
        //println!("shortening {long_url}");

        let group_guid = match &self.cfg.default_group_guid {
            Some(group_guid) => Cow::from(group_guid),
            None => match self.fetch_user().await? {
                User {
                    is_active: false, ..
                } => return Err(Error::UnknownGroupGUID("user is inactive")),
                User {
                    default_group_guid, ..
                } => Cow::Owned(default_group_guid),
            },
        };

        // TODO: cache links in a local sqlite DB
        //  - use e.g. `$XDG_CACHE_HOME/bitly/links`
        //  - add `--offline` mode
        let payload = Shorten {
            long_url,
            domain: self.cfg.domain.as_deref(),
            group_guid,
        };

        // TODO: check local cache for the payload

        //println!("sending shorten request: {payload:#?}");
        let resp = self
            .http
            .post("https://api-ssl.bitly.com/v4/shorten")
            .bearer_auth(self.cfg.api_token())
            .json(&payload)
            .send()
            .await?;

        parse_response! { resp =>
            OK | CREATED
            ||
            BAD_REQUEST
            | FORBIDDEN
            | EXPECTATION_FAILED
            | UNPROCESSABLE_ENTITY
            | TOO_MANY_REQUESTS
            | INTERNAL_SERVER_ERROR
            | SERVICE_UNAVAILABLE
        }
    }
}
