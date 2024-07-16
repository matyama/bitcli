use std::path::Path;
use std::str::FromStr as _;

use sqlx::prelude::*;
use sqlx::sqlite::{SqliteConnectOptions, SqliteRow};
use sqlx::SqlitePool;

use crate::api::{Bitlink, Shorten};
use crate::config::APP;

pub struct BitlinkCache {
    pool: sqlx::SqlitePool,
}

impl BitlinkCache {
    pub async fn new(name: &str, cache_dir: Option<impl AsRef<Path>>) -> Option<Self> {
        let cache_dir = match cache_dir {
            Some(dir) if dir.as_ref().as_os_str().is_empty() => return None,
            // XXX: allow relative paths (requires MSRV bump)
            // `std::path::absolute(cache_dir)` (note: `canonicalize` accesses FS => must exist)
            Some(cache_dir) => cache_dir.as_ref().to_path_buf(),
            None => xdg::BaseDirectories::with_prefix(APP)
                .map(|dirs| dirs.get_cache_home())
                .ok()?,
        };

        if !cache_dir.is_dir() {
            if let Err(error) = std::fs::create_dir_all(cache_dir.as_path()) {
                // TODO: add a log macro
                eprintln!("{APP}(cache): {error}");
                return None;
            };
        }

        if !cache_dir.is_dir() {
            eprintln!("{APP}(cache): 'cache_dir' must be a directory");
            return None;
        }

        let path = cache_dir.join(format!("{name}.db"));
        let path = path.to_string_lossy();

        // TODO: add a log macro
        //println!("using cache {path:?}");

        let Ok(ops) = SqliteConnectOptions::from_str(&format!("sqlite:{path}")) else {
            eprintln!("{APP}(cache): invalid database path {path:?}");
            return None;
        };

        let ops = ops.create_if_missing(true);

        let pool = match SqlitePool::connect_with(ops).await {
            Ok(pool) => pool,
            Err(err) => {
                eprintln!("{APP}(cache): {err}");
                return None;
            }
        };

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

        if let Err(err) = res {
            // TODO: add a log macro
            eprintln!("{APP}(cache-create): {err}");
            return None;
        }

        Some(Self { pool })
    }

    pub async fn get(&self, query: &Shorten<'_>) -> Option<Bitlink> {
        let res = sqlx::query_as(
            r#"
            SELECT id, link
            FROM shorten
            WHERE group_guid = $1 AND domain = $2 AND long_url = $3
            LIMIT 1
            "#,
        )
        .bind(query.group_guid.as_ref())
        .bind(query.domain)
        .bind(query.long_url.as_str())
        .fetch_optional(&self.pool)
        .await;

        match res {
            Ok(link) => link,
            Err(err) => {
                // TODO: add a log macro
                eprintln!("{APP}(cache-get): {err}");
                None
            }
        }
    }

    pub async fn set(&self, query: Shorten<'_>, link: &Bitlink) {
        let res = sqlx::query(
            r#"
            INSERT INTO shorten (id, link, long_url, domain, group_guid) VALUES
            ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(&link.id)
        .bind(link.link.as_str())
        .bind(query.long_url.as_str())
        .bind(query.domain)
        .bind(query.group_guid.as_ref())
        .execute(&self.pool)
        .await;

        match res {
            Ok(res) => debug_assert_eq!(res.rows_affected(), 1),
            Err(error) => {
                // TODO: add a log macro
                eprintln!("{APP}(cache-set): {error}");
            }
        }
    }
}

impl FromRow<'_, SqliteRow> for Bitlink {
    fn from_row(row: &SqliteRow) -> sqlx::Result<Self> {
        Ok(Self {
            link: row.try_from::<&str, _, _>("link")?,
            id: row.try_get("id")?,
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
