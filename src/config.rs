use std::borrow::Cow;
use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use hide::Hide;
use serde::Deserialize;
use url::Url;

pub const APP: &str = "bitcli";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Load(#[from] config::ConfigError),

    #[error(transparent)]
    Xdg(#[from] xdg::BaseDirectoriesError),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default = "default::api_url")]
    pub api_url: Url,

    /// API access token
    pub api_token: Hide<String>,

    /// The domain to create bitlinks under (defaults to `bit.ly` if unspecified)
    pub domain: Option<String>,

    /// Default group GUID used in shorten requests (optional)
    ///
    /// If unspecified, the group GUID resolution before making a request is in order as follows:
    ///  1. Value stored and loaded from a config file
    ///  2. Value given as a program argument or an environment variable
    ///  3. Fetch current default group GUID for the logged in user
    pub default_group_guid: Option<String>,

    /// Path to the cache directory
    ///
    /// If set to an empty path, then caching will be disabled.
    pub cache_dir: Option<PathBuf>,

    /// If set to `true` then no API requests will be issued (disabled by default)
    ///
    /// Any command will only rely on the local cache under the _offline_ mode.
    #[serde(default = "default::offline")]
    pub offline: bool,

    /// Maximum number of API requests in flight
    #[serde(default = "default::max_concurrent")]
    pub max_concurrent: usize,
}

impl Config {
    pub fn load(config: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let config = config.as_ref();
        let cfg_dir = get_config_dir(config)?;

        let cfg = config::Config::builder()
            .add_source(config::File::with_name(config.to_string_lossy().as_ref()));

        let Imports { import } = cfg
            .build_cloned()
            .and_then(config::Config::try_deserialize)?;

        let cfg = import
            .into_iter()
            .filter_map(|path| resolve_import_path(&cfg_dir, path))
            .fold(cfg, |builder, path| {
                builder.add_source(config::File::with_name(path.to_string_lossy().as_ref()))
            })
            .build()?;

        cfg.try_deserialize().map_err(ConfigError::Load)
    }

    /// Update current configs with _some_ of the given options (only those that are `Some`)
    pub fn override_with(&mut self, ops: impl Into<Options>) {
        let ops = ops.into();

        if ops.domain.is_some() {
            self.domain = ops.domain;
        }

        if ops.group_guid.is_some() {
            self.default_group_guid = ops.group_guid;
        }

        if ops.cache_dir.is_some() {
            self.cache_dir = ops.cache_dir;
        }

        if let Some(offline) = ops.offline {
            self.offline = offline;
        }

        if let Some(max_concurrent) = ops.max_concurrent {
            self.max_concurrent = max_concurrent.into();
        }
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn with_api_url(&mut self, api_url: Url) {
        self.api_url = api_url;
    }

    #[inline]
    pub(crate) fn api_token(&self) -> &str {
        self.api_token.as_ref()
    }
}

mod default {
    use url::Url;

    #[inline]
    pub(super) fn api_url() -> Url {
        Url::parse("https://api-ssl.bitly.com").expect("valid API URL")
    }

    #[inline]
    pub(super) fn offline() -> bool {
        false
    }

    #[inline]
    pub(super) fn max_concurrent() -> usize {
        16
    }
}

#[derive(Debug, Default)]
pub struct Options {
    /// The domain to create bitlinks under (defaults to `bit.ly` if unspecified)
    pub domain: Option<String>,

    /// Default group GUID used in shorten requests (optional)
    pub group_guid: Option<String>,

    /// Alternative path to the cache directory
    pub cache_dir: Option<PathBuf>,

    /// Controls whether issuing API requests is allowed
    pub offline: Option<bool>,

    /// Maximum number of API requests in flight
    pub max_concurrent: Option<NonZeroUsize>,
}

#[derive(Debug, Deserialize)]
#[serde(bound = "'de: 'a")]
struct Imports<'a> {
    import: Vec<Cow<'a, Path>>,
}

fn resolve_import_path(cfg_dir: impl AsRef<Path>, path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = path.as_ref();

    #[cfg(target_family = "unix")]
    let path = match path.strip_prefix("~") {
        Ok(home_relative) => {
            let mut home = home::home_dir()?;
            home.push(home_relative);
            home
        }
        Err(_) if path.is_relative() => cfg_dir.as_ref().join(path),
        Err(_) => path.to_path_buf(),
    };

    #[cfg(not(target_family = "unix"))]
    let path = if path.is_relative() {
        cfg_dir.as_ref().join(path)
    } else {
        path.to_path_buf()
    };

    path.canonicalize().ok()
}

fn get_config_dir(cfg_path: &Path) -> io::Result<Cow<'_, Path>> {
    cfg_path
        .parent()
        .ok_or_else(|| io::Error::other("config file has no parent directory"))
        .and_then(|path| {
            if path.as_os_str().is_empty() {
                std::env::current_dir().map(Cow::Owned)
            } else {
                Ok(Cow::from(path))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    use std::io::Write as _;

    use tempfile::{Builder, NamedTempFile, TempDir};

    #[fixture]
    #[once]
    fn config_dir() -> TempDir {
        TempDir::new().expect("test temp dir")
    }

    #[fixture]
    fn config_file(config_dir: &TempDir) -> NamedTempFile {
        Builder::new()
            .suffix(".toml")
            .tempfile_in(config_dir)
            .expect("temp config file")
    }

    #[fixture]
    fn import_file(config_dir: &TempDir) -> NamedTempFile {
        Builder::new()
            .suffix(".toml")
            .tempfile_in(config_dir)
            .expect("temp import file")
    }

    #[fixture]
    fn config() -> Config {
        Config {
            api_url: default::api_url(),
            api_token: "test-api-token".into(),
            domain: None,
            default_group_guid: None,
            cache_dir: None,
            offline: default::offline(),
            max_concurrent: default::max_concurrent(),
        }
    }

    #[rstest]
    fn load_config(mut config_file: NamedTempFile, mut import_file: NamedTempFile) {
        write!(
            import_file,
            r#"
            # API token
            api_token = "test-api-token"

            # Default group GUID
            default_group_guid = "test-group-guid"
            "#,
        )
        .expect("write temp auth file");

        write!(
            config_file,
            r#"
            import = [{:?}]

            # Cache directory (optional, empty path disables caching)
            cache_dir = ""

            # Maximum number of API requests in flight (default: 16)
            max_concurrent = 8
            "#,
            import_file.path()
        )
        .expect("write temp config file");

        let expected = Config {
            api_url: default::api_url(),
            api_token: "test-api-token".into(),
            domain: None,
            default_group_guid: Some("test-group-guid".to_string()),
            cache_dir: Some(PathBuf::new()),
            offline: false,
            max_concurrent: 8,
        };

        match Config::load(config_file) {
            Ok(actual) => assert_eq!(expected, actual),
            Err(error) => panic!("expected to read valid test config, got: {error:?}"),
        }
    }

    #[rstest]
    fn config_file_does_not_exist() {
        let Err(error) = Config::load(PathBuf::from("/tmp/non-existant.toml")) else {
            panic!("loaded config from non-existent file");
        };

        match error {
            ConfigError::Load(_) => {}
            error => panic!("expected to get a Load error, got: {error:?}"),
        }
    }

    #[rstest]
    fn ignore_missing_imports(mut config_file: NamedTempFile, #[from(config)] expected: Config) {
        write!(
            config_file,
            r#"
            import = ["non-existent"]
            api_token = "test-api-token"
            "#,
        )
        .expect("write temp config file");

        match Config::load(config_file) {
            Ok(actual) => assert_eq!(expected, actual),
            Err(error) => panic!("expected to read valid test config, got: {error:?}"),
        }
    }

    #[rstest]
    fn load_fails_when_missing_required_fields(mut config_file: NamedTempFile) {
        // NOTE: missing `api_token` & it can't be found in the non-existent import
        write!(
            config_file,
            r#"
            import = ["non-existent"]
            "#,
        )
        .expect("write temp config file");

        let Err(error) = Config::load(config_file) else {
            panic!("loaded config that is missing a required field: 'api_token'");
        };

        match error {
            ConfigError::Load(_) => {}
            error => panic!("expected to get a Load error, got: {error:?}"),
        }
    }

    #[rstest]
    fn override_options(mut config: Config) {
        config.override_with(Options {
            domain: Some("my-domain".to_string()),
            group_guid: None,
            cache_dir: None,
            offline: None,
            max_concurrent: None,
        });

        config.override_with(Options {
            domain: None,
            group_guid: None,
            cache_dir: None,
            offline: Some(true),
            max_concurrent: None,
        });

        let expected = Config {
            api_url: default::api_url(),
            api_token: "test-api-token".into(),
            domain: Some("my-domain".to_string()),
            default_group_guid: None,
            cache_dir: None,
            offline: true,
            max_concurrent: default::max_concurrent(),
        };

        assert_eq!(expected, config);
    }
}
