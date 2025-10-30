use std::path::Path;
use std::str::FromStr as _;

use sqlx::prelude::*;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqliteRow};
use tracing::{debug, error, instrument};

use crate::api::{Bitlink, Shorten};
use crate::config::APP;

#[derive(Debug)]
pub struct BitlinkCache {
    pool: SqlitePool,
}

impl BitlinkCache {
    #[instrument(name = "init_cache", level = "debug", skip(cache_dir))]
    pub async fn new(name: &str, cache_dir: Option<impl AsRef<Path>>) -> Option<Self> {
        let cache_dir = match cache_dir {
            Some(dir) if dir.as_ref().as_os_str().is_empty() => return None,
            Some(cache_dir) => std::path::absolute(cache_dir).ok()?,
            None => xdg::BaseDirectories::with_prefix(APP).get_cache_home()?,
        };

        if !cache_dir.is_dir()
            && let Err(error) = std::fs::create_dir_all(cache_dir.as_path())
        {
            error!(%error, "failed to create cache directory");
            return None;
        }

        if !cache_dir.is_dir() {
            error!(?cache_dir, "'cache_dir' must be a directory");
            return None;
        }

        let path = cache_dir.join(format!("{name}.db"));
        let path = path.to_string_lossy();

        let Ok(ops) = SqliteConnectOptions::from_str(&format!("sqlite:{path}")) else {
            error!(?path, "invalid database path");
            return None;
        };

        let ops = ops.create_if_missing(true);

        debug!(%path, "connecting to SQLite database");

        let pool = match SqlitePool::connect_with(ops).await {
            Ok(pool) => pool,
            Err(error) => {
                error!(%error, "database connection failed");
                return None;
            }
        };

        debug!("setting up database tables");

        let res = sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS shorten (
              id TEXT NOT NULL UNIQUE,
              link TEXT NOT NULL,
              long_url TEXT NOT NULL,
              domain TEXT,
              group_guid TEXT NOT NULL
            );

            CREATE UNIQUE INDEX IF NOT EXISTS ix_shorten
            ON shorten (group_guid, domain, long_url);
            "#,
        )
        .execute(&pool)
        .await;

        if let Err(error) = res {
            error!(%error, "failed to set up database");
            return None;
        }

        Some(Self { pool })
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn get(&self, query: &Shorten<'_>) -> Option<Bitlink> {
        debug!("checking local cache");

        let res = sqlx::query_as(
            r#"
            SELECT id, link, long_url
            FROM shorten
            WHERE group_guid = $1 AND domain = $2 AND long_url = $3
            LIMIT 1
            "#,
        )
        .bind(query.group_guid.as_ref())
        .bind(query.domain.as_ref())
        .bind(query.long_url.as_str())
        .fetch_optional(&self.pool)
        .await;

        match res {
            Ok(link) => link,
            Err(error) => {
                error!(%error, "failed to access local cache");
                None
            }
        }
    }

    #[instrument(level = "debug", skip(self), ret)]
    pub async fn set(&self, query: &Shorten<'_>, link: &Bitlink) -> bool {
        debug!("updating local cache");

        let res = sqlx::query(
            r#"
            INSERT INTO shorten (id, link, long_url, domain, group_guid) VALUES
            ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(&link.id)
        .bind(link.link.as_str())
        .bind(query.long_url.as_str())
        .bind(query.domain.as_ref())
        .bind(query.group_guid.as_ref())
        .execute(&self.pool)
        .await;

        match res {
            Ok(res) => res.rows_affected() == 1,
            Err(error) => {
                error!(%error, "failed to update local cache");
                false
            }
        }
    }
}

impl FromRow<'_, SqliteRow> for Bitlink {
    fn from_row(row: &SqliteRow) -> sqlx::Result<Self> {
        Ok(Self {
            link: row.try_from::<&str, _, _>("link")?,
            id: row.try_get("id")?,
            long_url: row.try_from::<&str, _, _>("long_url")?,
        })
    }
}

trait RowExt: Row {
    fn try_from<'r, T, I, R>(&'r self, index: I) -> sqlx::Result<R>
    where
        T: Decode<'r, Self::Database> + Type<Self::Database>,
        I: sqlx::ColumnIndex<Self> + std::fmt::Display,
        R: TryFrom<T>,
        <R as TryFrom<T>>::Error: std::error::Error + Send + Sync + 'static;
}

impl RowExt for SqliteRow {
    fn try_from<'r, T, I, R>(&'r self, index: I) -> sqlx::Result<R>
    where
        T: Decode<'r, Self::Database> + Type<Self::Database>,
        I: sqlx::ColumnIndex<Self> + std::fmt::Display,
        R: TryFrom<T>,
        <R as TryFrom<T>>::Error: std::error::Error + Send + Sync + 'static,
    {
        self.try_get(&index).and_then(|val| {
            R::try_from(val).map_err(|source| sqlx::Error::ColumnDecode {
                index: index.to_string(),
                source: Box::new(source),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::path::PathBuf;

    use super::*;
    use rstest::*;

    use tempfile::TempDir;

    #[fixture]
    fn shorten<'a>() -> Shorten<'a> {
        Shorten {
            long_url: "https://example.com".parse().unwrap(),
            domain: Some(Cow::Borrowed("bit.ly")),
            group_guid: Cow::Borrowed("test-group-guid"),
        }
    }

    #[fixture]
    fn link() -> Bitlink {
        Bitlink {
            link: "https://bit.ly/4ePsyXN".parse().unwrap(),
            id: "some-bitlink-id".to_string(),
            long_url: "https://example.com".parse().unwrap(),
        }
    }

    #[fixture]
    fn cache_dir() -> TempDir {
        tempfile::tempdir().expect("failed to create temp cache dir")
    }

    #[fixture]
    async fn cache(cache_dir: TempDir, #[default("test")] name: &str) -> BitlinkCache {
        let path = cache_dir.path().to_path_buf();

        let Some(cache) = BitlinkCache::new(name, Some(cache_dir)).await else {
            panic!("failed to create new '{name}' cache in {path:?}");
        };

        cache
    }

    #[rstest]
    #[tokio::test]
    async fn set_is_idempotent(
        #[future(awt)] cache: BitlinkCache,
        shorten: Shorten<'_>,
        link: Bitlink,
    ) {
        let set = cache.set(&shorten, &link).await;
        assert!(set, "cache set should succeed on unique entry");

        let set = cache.set(&shorten, &link).await;
        assert!(!set, "cache set should ignore an existing entry");
    }

    #[rstest]
    #[tokio::test]
    async fn get_non_existent_yield_nothing(
        #[future(awt)] cache: BitlinkCache,
        shorten: Shorten<'_>,
    ) {
        let link = cache.get(&shorten).await;
        assert!(link.is_none(), "expected empty cache, got {link:?}");
    }

    #[rstest]
    #[tokio::test]
    async fn set_get_is_id_link(
        #[future(awt)] cache: BitlinkCache,
        shorten: Shorten<'static>,
        link: Bitlink,
    ) {
        cache.set(&shorten, &link).await;
        let cached = cache.get(&shorten).await;

        assert_eq!(Some(link), cached);
    }

    #[rstest]
    #[tokio::test]
    async fn disable_cache() {
        let cache = BitlinkCache::new("test-disable-cache", Some(PathBuf::new())).await;
        assert!(cache.is_none(), "empty cache dir should disable the cache");
    }

    #[cfg(target_family = "unix")]
    #[rstest]
    #[tokio::test]
    async fn relative_cache_dir(cache_dir: TempDir) {
        let dir_name = cache_dir
            .path()
            .file_name()
            .map(Path::new)
            .expect("temp dir name");

        let mut relative_cache_dir = PathBuf::from("~/../../tmp");
        relative_cache_dir.push(dir_name);

        // FIXME: test is not well isolated, it creates ~/../../tmp/<tempdir>/...
        let cache = BitlinkCache::new("test-relative-cache-dir", Some(relative_cache_dir)).await;

        assert!(
            cache.is_some(),
            "relative cache dir path should be resolved"
        );
    }
}
