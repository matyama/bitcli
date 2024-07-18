[![pre-commit](https://img.shields.io/badge/pre--commit-enabled-brightgreen?logo=pre-commit)](https://github.com/pre-commit/pre-commit)

# bitcli
Simple CLI tool for URL shortening via Bitly

Setup a config file with your Bitly API token and run:
```console
$ bitcli https://example.com
https://bit.ly/4ePsyXN
```

## Installation

### Cargo
**Requirements**: `rustc >= 1.74`

```bash
cargo install --locked bitcli
```

**Note**: Until published to [Crates.io](https://crates.io), install
from git source.
```bash
cargo install --locked --git https://github.com/matyama/bitcli.git
```

## Configuration
The configuration is a TOML file and at minimum must contain an
`api_token` string.

Example config file `$XDG_CONFIG_HOME/bitcli/config.toml` which uses an
import for sensitive information (auth info):
```toml
import = ["auth.toml"]

# Cache directory (optional, empty path disables caching)
# cache_dir = "/path/to/cache/bitcli"

# Default domain (optional)
domain = "bit.ly"
```

Imports can be either absolute paths, or relative to the directory of
the main config file (or relative to the home directory using `~`).

For instance, in the example above, one would create a protected
credentials file `$XDG_CONFIG_HOME/bitcli/auth.toml` with
```toml
# API token
api_token = "<API TOKEN>"

# Default group GUID (optional)
default_group_guid = "<DEFAULT GROUP GUID>"

# Maximum number of API requests in flight (default: 16)
max_concurrent = 16
```

Then you can read-protect just a portion of the config
(e.g., `chmod 600 auth.toml`) and share the rest.
