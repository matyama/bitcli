use std::borrow::Cow;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueHint};
use url::Url;

use crate::config::{ConfigError, Options, APP};

#[derive(Debug, Parser)]
#[command(name = APP)]
#[command(version, about, long_about = None)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Alternative path to the config file (TOML)
    #[arg(short, long, env = "BITCLI_CONFIG_FILE", value_hint = ValueHint::FilePath)]
    config_file: Option<PathBuf>,

    /// Alternative path to the cache directory
    ///
    /// If set to an empty path, then caching will be disabled.
    #[arg(long, env = "BITCLI_CACHE_DIR", value_hint = ValueHint::DirPath)]
    cache_dir: Option<PathBuf>,

    /// Explicitly disable local cache for this command invocation
    ///
    /// Equivalent to passing an empty `--cache-dir` path. Takes priority over `--cache-dir`.
    #[arg(
        long,
        default_value = "false",
        overrides_with = "cache_dir",
        env = "BITCLI_NO_CACHE"
    )]
    no_cache: bool,

    // emulate default (sub)command
    #[clap(flatten)]
    shorten: ShortenArgs,
}

impl Cli {
    /// Get the location of the config file
    ///
    /// Note that if `--config-file` has not been specified, then this will look for `config.toml`
    /// under the XDG base directories (e.g., `$XDG_CONFIG_HOME/bitcli/` or `~/.config/bitcli/`).
    pub fn config_file(&self) -> Result<Cow<'_, Path>, ConfigError> {
        match &self.config_file {
            Some(config) => Ok(Cow::from(config)),
            None => xdg::BaseDirectories::with_prefix(APP)?
                .find_config_file("config.toml")
                .map(Cow::Owned)
                .ok_or_else(|| std::io::Error::other("missing config.toml"))
                .map_err(ConfigError::Io),
        }
    }
}

impl From<&Cli> for Options {
    fn from(cli: &Cli) -> Self {
        let mut ops = Self::default();

        if cli.no_cache {
            // NOTE: empty path for the `cache_dir` disables the cache
            ops.cache_dir = Some(PathBuf::new());
        } else {
            ops.cache_dir.clone_from(&cli.cache_dir);
        }

        ops
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Shorten URL and print the result to the output (default)")]
    Shorten(ShortenArgs),
}

impl From<Cli> for Command {
    #[inline]
    fn from(cli: Cli) -> Self {
        cli.command.unwrap_or(Self::Shorten(cli.shorten))
    }
}

impl From<&Command> for Options {
    fn from(cmd: &Command) -> Self {
        let mut ops = Self::default();

        match cmd {
            Command::Shorten(ShortenArgs {
                domain, group_guid, ..
            }) => {
                ops.domain.clone_from(domain);
                ops.group_guid.clone_from(group_guid);
            }
        }

        ops
    }
}

#[derive(Args, Debug)]
pub struct ShortenArgs {
    // TODO: allow multiple
    // TODO: allow reading URL(s) from stdin
    /// URL to shorten
    pub url: Url,

    /// The domain to create bitlinks under
    #[arg(short, long, env = "BITCLI_DOMAIN")]
    pub domain: Option<String>,

    /// The group GUID to create bitlinks under
    ///
    /// If unspecified, the resolution is as follows (latter overriding the former):
    ///
    ///  1. Value stored and loaded from a config file
    ///
    ///  2. Value given as a program argument or an environment variable
    ///
    ///  3. If still unknown, fetch current default group GUID for the authenticated user
    #[arg(short, long, env = "BITCLI_GROUP_GUID")]
    pub group_guid: Option<String>,
}
