use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};

use hide::Hide;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Load(#[from] config::ConfigError),

    #[error(transparent)]
    Xdg(#[from] xdg::BaseDirectoriesError),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Deserialize)]
pub struct Config {
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
        let Options { domain, group_guid } = ops.into();

        if domain.is_some() {
            self.domain = domain;
        }

        if group_guid.is_some() {
            self.default_group_guid = group_guid;
        }
    }

    #[inline]
    pub(crate) fn api_token(&self) -> &str {
        self.api_token.as_ref()
    }
}

#[derive(Debug, Default)]
pub struct Options {
    /// The domain to create bitlinks under (defaults to `bit.ly` if unspecified)
    pub domain: Option<String>,

    /// Default group GUID used in shorten requests (optional)
    pub group_guid: Option<String>,
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
