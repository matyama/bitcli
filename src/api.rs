use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;

use futures_util::stream::{Stream, StreamExt as _};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};
use url::Url;

use crate::cache::BitlinkCache;
use crate::cli::Ordering;
use crate::config::Config;
use crate::error::{Error, Result};

const VERSION: &str = "v4";

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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Bitlink {
    pub link: Url,
    pub id: String,
    pub long_url: Url,
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

fn api_url(base: &Url, endpoint: &str) -> Url {
    let mut api_url = base.clone();
    api_url.set_path(&format!("{VERSION}/{endpoint}"));
    api_url
}

struct ClientInner {
    cfg: Config,
    http: Option<reqwest::Client>,
    cache: Option<BitlinkCache>,
}

impl ClientInner {
    #[inline]
    fn api_url(&self, endpoint: &str) -> Url {
        api_url(&self.cfg.api_url, endpoint)
    }

    #[instrument(level = "debug", skip(self))]
    async fn fetch_user(&self) -> Result<User> {
        let Some(ref http) = self.http else {
            return Err(Error::Offline("user"));
        };

        let endpoint = self.api_url("user");

        debug!("fetching user info");
        let resp = http
            .get(endpoint)
            .bearer_auth(self.cfg.api_token())
            .send()
            .await?;

        parse_response! { resp =>
            OK
            ||
            FORBIDDEN
            | GONE
            | NOT_FOUND
            | INTERNAL_SERVER_ERROR
            | SERVICE_UNAVAILABLE
        }
    }

    #[instrument(level = "debug", fields(%long_url), skip_all)]
    async fn shorten(&self, long_url: Url) -> Result<Bitlink> {
        debug!("shortening URL");

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

        let payload = Shorten {
            long_url,
            domain,
            group_guid,
        };

        // fast path: check local cache for the bitlink
        if let Some(ref cache) = self.cache
            && let Some(bitlink) = cache.get(&payload).await
        {
            return Ok(bitlink);
        }

        let Some(ref http) = self.http else {
            return Err(Error::Offline("shorten"));
        };

        let endpoint = self.api_url("shorten");

        debug!(?payload, "sending shorten request");

        let resp = http
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
            | GONE
            | EXPECTATION_FAILED
            | UNPROCESSABLE_ENTITY
            | TOO_MANY_REQUESTS
            | INTERNAL_SERVER_ERROR
            | SERVICE_UNAVAILABLE
        };

        // if successful then update local cache
        if let Ok(ref result) = result
            && let Some(ref cache) = self.cache
        {
            cache.set(&payload, result).await;
        }

        result
    }

    #[instrument(level = "debug", skip_all)]
    fn shorten_all(
        self: Arc<Self>,
        urls: impl Stream<Item = Url>,
    ) -> impl Stream<Item = impl Future<Output = Result<Bitlink>>> {
        urls.map(move |url| {
            let client = Arc::clone(&self);
            async move { client.shorten(url).await }
        })
    }
}

pub struct Client {
    inner: Arc<ClientInner>,
}

// TODO: handle timeouts, cancellation, API limits (see `GET /v4/user/platform_limits`), etc.
impl Client {
    #[instrument(name = "init_client", level = "debug")]
    pub async fn new(cfg: Config) -> Self {
        let http = if cfg.offline {
            debug!("offline mode enabled, skipping HTTP client initialization");
            None
        } else {
            debug!("initializing HTTP client");
            Some(reqwest::Client::new())
        };

        let cache = BitlinkCache::new(VERSION, cfg.cache_dir.as_ref()).await;

        Self {
            inner: Arc::new(ClientInner { cfg, http, cache }),
        }
    }

    #[instrument(level = "debug", skip(self, urls))]
    pub fn shorten<'a, S>(
        &self,
        urls: S,
        ordering: Ordering,
    ) -> impl Stream<Item = Result<Bitlink>> + 'a
    where
        S: Stream<Item = Url> + Send + 'a,
    {
        let client = Arc::clone(&self.inner);
        let max_concurrent = client.cfg.max_concurrent;

        match ordering {
            Ordering::Ordered => client
                .shorten_all(urls)
                .buffered(max_concurrent)
                .left_stream(),

            Ordering::Unordered => client
                .shorten_all(urls)
                .buffer_unordered(max_concurrent)
                .right_stream(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;

    use futures_util::stream;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

    struct LinkResponder {
        resp_num: AtomicUsize,
        responses: Vec<String>,
    }

    impl LinkResponder {
        fn new(ordering: Ordering, responses: impl IntoIterator<Item = impl Into<String>>) -> Self {
            let mut responses = responses.into_iter().map(Into::<String>::into).collect();
            Self {
                resp_num: AtomicUsize::new(0),
                responses: match ordering {
                    Ordering::Ordered => responses,
                    Ordering::Unordered => {
                        responses.reverse();
                        responses
                    }
                },
            }
        }
    }

    impl Respond for LinkResponder {
        fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
            let i = self
                .resp_num
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            let body = self.responses[i].as_bytes();

            ResponseTemplate::new(StatusCode::OK).set_body_raw(body, "application/json")
        }
    }

    struct ShortenTest {
        urls: Vec<Url>,
        responder: LinkResponder,
        expected: Vec<Bitlink>,
    }

    struct ServerConfig {
        server: MockServer,
        config: Config,
    }

    // NOTE: starts mock server on a random local port
    #[fixture]
    async fn server() -> MockServer {
        MockServer::start().await
    }

    // NOTE: disables caching
    #[fixture]
    fn config() -> Config {
        Config {
            api_url: Url::parse("https://api-ssl.bitly.com").unwrap(),
            api_token: "secret-token".into(),
            domain: Some("test.domain".to_string()),
            default_group_guid: Some("test-group-guid".to_string()),
            cache_dir: Some(PathBuf::new()),
            offline: false,
            max_concurrent: 4,
        }
    }

    #[fixture]
    async fn server_config(#[future(awt)] server: MockServer, mut config: Config) -> ServerConfig {
        config.with_api_url(server.uri().parse().expect("valid mock API URL"));
        ServerConfig { server, config }
    }

    #[fixture]
    fn urls() -> Vec<Url> {
        vec![
            Url::parse("https://example.com").unwrap(),
            Url::parse("http://example.com").unwrap(),
        ]
    }

    #[fixture]
    fn shorten_test(
        #[default(Ordering::Ordered)] ordering: Ordering,
        urls: Vec<Url>,
    ) -> ShortenTest {
        ShortenTest {
            urls,
            responder: LinkResponder::new(
                ordering,
                [
                    r#"{
                      "created_at": "2024-08-07T08:48:48+0000",
                      "id": "1",
                      "link": "https://test.domain/4ePsyXN",
                      "custom_bitlinks": [],
                      "long_url": "https://example.com",
                      "archived": false,
                      "tags": [],
                      "deeplinks": [],
                      "references": {
                        "group": "https://api-ssl.bitly.com/v4/groups/test-group-guid"
                      }
                    }"#,
                    r#"{
                      "created_at": "2024-08-07T08:48:49+0000",
                      "id": "2",
                      "link": "https://test.domain/3WA1XXp",
                      "custom_bitlinks": [],
                      "long_url": "http://example.com",
                      "archived": false,
                      "tags": [],
                      "deeplinks": [],
                      "references": {
                        "group": "https://api-ssl.bitly.com/v4/groups/test-group-guid"
                      }
                    }"#,
                ],
            ),
            expected: vec![
                Bitlink {
                    link: "https://test.domain/4ePsyXN".parse().unwrap(),
                    id: "1".to_string(),
                    long_url: "https://example.com".parse().unwrap(),
                },
                Bitlink {
                    link: "https://test.domain/3WA1XXp".parse().unwrap(),
                    id: "2".to_string(),
                    long_url: "http://example.com".parse().unwrap(),
                },
            ],
        }
    }

    async fn test_shorten(
        config: Config,
        urls: Vec<Url>,
        ordering: Ordering,
    ) -> Vec<Result<Bitlink>> {
        // TODO: parametrize client by cache to be able to mock it for tests
        let client = Client::new(config).await;
        client
            .shorten(stream::iter(urls), ordering)
            .collect::<Vec<_>>()
            .await
    }

    #[rstest]
    #[tokio::test]
    async fn shorten_urls_ordered(
        #[future(awt)] server_config: ServerConfig,
        #[from(shorten_test)] ShortenTest {
            urls,
            responder,
            expected,
        }: ShortenTest,
    ) {
        let ServerConfig { server, config } = server_config;

        Mock::given(method("POST"))
            .and(path("v4/shorten"))
            .respond_with(responder)
            .mount(&server)
            .await;

        let results = test_shorten(config, urls, Ordering::Ordered).await;

        match results.into_iter().collect::<Result<Vec<_>>>() {
            Ok(actual) => assert_eq!(expected, actual),
            Err(error) => panic!("encountered API/client error: {error:?}"),
        }
    }

    #[rstest]
    #[tokio::test]
    async fn shorten_urls_unordered(
        #[future(awt)] server_config: ServerConfig,
        #[from(shorten_test)]
        #[with(Ordering::Unordered)]
        ShortenTest {
            urls,
            responder,
            expected,
        }: ShortenTest,
    ) {
        let ServerConfig { server, config } = server_config;

        Mock::given(method("POST"))
            .and(path("v4/shorten"))
            .respond_with(responder)
            .mount(&server)
            .await;

        let results = test_shorten(config, urls, Ordering::Unordered).await;

        match results.into_iter().collect::<Result<Vec<_>>>() {
            Ok(mut actual) => {
                actual.sort_by_cached_key(|link| link.id.parse::<u8>().expect("use u8 IDs"));
                assert_eq!(expected, actual)
            }
            Err(error) => panic!("encountered API/client error: {error:?}"),
        }
    }

    #[rstest]
    #[tokio::test]
    async fn shorten_auth_error(#[future(awt)] server_config: ServerConfig, urls: Vec<Url>) {
        let ServerConfig { server, config } = server_config;

        let forbidden = ResponseTemplate::new(StatusCode::FORBIDDEN)
            .set_body_raw(r#"{"message": "FORBIDDEN"}"#, "application/json");

        Mock::given(method("POST"))
            .and(path("v4/shorten"))
            .respond_with(forbidden)
            .mount(&server)
            .await;

        let results = test_shorten(config, urls, Ordering::Ordered).await;

        match results.into_iter().collect::<Result<Vec<_>>>() {
            Ok(links) => panic!("expected API error (FORBIDDEN), got: {links:?}"),
            Err(Error::Bitly(resp)) => assert_eq!("FORBIDDEN", resp.message),
            Err(error) => panic!("expected API error (FORBIDDEN), got: {error:?}"),
        }
    }

    // TODO: test with caching enabled and --offline

    #[rstest]
    #[case::shorten(
        "https://api-ssl.bitly.com",
        "shorten",
        "https://api-ssl.bitly.com/v4/shorten"
    )]
    fn make_api_url(#[case] base: &str, #[case] endpoint: &str, #[case] expected: &str) {
        let base = Url::parse(base).unwrap();
        let expected = Url::parse(expected).unwrap();
        let actual = api_url(&base, endpoint);
        assert_eq!(expected, actual);
    }
}
