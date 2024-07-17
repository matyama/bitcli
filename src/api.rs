use std::borrow::Cow;
use std::sync::OnceLock;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::cache::BitlinkCache;
use crate::config::Config;
use crate::error::{Error, Result};

const VERSION: &str = "v4";

fn api_url(endpoint: &str) -> Url {
    static API_URL: OnceLock<Url> = OnceLock::new();
    API_URL
        .get_or_init(|| {
            format!("https://api-ssl.bitly.com/{VERSION}/")
                .parse()
                .expect("invalid API URL")
        })
        .join(endpoint)
        .expect("invalid endpoint URL")
}

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
pub struct Shorten<'a> {
    pub long_url: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<Cow<'a, str>>,
    pub group_guid: Cow<'a, str>,
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

#[derive(Debug, Deserialize, PartialEq, Eq)]
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
    ($resp:expr => $ok:ident $(| $oks:ident)* || $err:ident $(| $errs:ident)*) => {{
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
        }}
    };
}

pub struct Client {
    cfg: Config,
    http: reqwest::Client,
    cache: Option<BitlinkCache>,
}

// TODO: handle timeouts, cancellation, API limits (see `GET /v4/user/platform_limits`), etc.
impl Client {
    pub async fn new(cfg: Config) -> Self {
        let http = reqwest::Client::new();
        let cache = BitlinkCache::new(VERSION, cfg.cache_dir.as_ref()).await;
        Self { cfg, http, cache }
    }

    pub async fn fetch_user(&self) -> Result<User> {
        let endpoint = api_url("user");

        //println!("fetching user info");
        let resp = self
            .http
            .get(endpoint)
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

        let domain = self.cfg.domain.as_deref().map(Cow::Borrowed);

        // TODO: cache links in a local sqlite DB
        //  - use e.g. `$XDG_CACHE_HOME/bitly/links`
        //  - add `--offline` mode (possibly conflicts with `--no-cache`)
        let payload = Shorten {
            long_url,
            domain,
            group_guid,
        };

        // fast path: check local cache for the bitlink
        if let Some(ref cache) = self.cache {
            if let Some(bitlink) = cache.get(&payload).await {
                return Ok(bitlink);
            }
        }

        let endpoint = api_url("shorten");

        //println!("sending shorten request: {payload:#?}");
        let resp = self
            .http
            .post(endpoint)
            .bearer_auth(self.cfg.api_token())
            .json(&payload)
            .send()
            .await?;

        let result = parse_response! { resp =>
            OK | CREATED
            ||
            BAD_REQUEST
            | FORBIDDEN
            | EXPECTATION_FAILED
            | UNPROCESSABLE_ENTITY
            | TOO_MANY_REQUESTS
            | INTERNAL_SERVER_ERROR
            | SERVICE_UNAVAILABLE
        };

        // if successful then update local cache
        if let Ok(ref result) = result {
            if let Some(ref cache) = self.cache {
                cache.set(&payload, result).await;
            }
        }

        result
    }
}
