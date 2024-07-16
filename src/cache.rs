use std::borrow::Cow;
use std::path::Path;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{named_params, Connection};
use url::Url;

use crate::api::{Bitlink, Shorten};
use crate::config::APP;

pub struct BitlinkCache {
    conn: Connection,
}

// TODO: all the cache operations are essentially blocking, make them async (e.g., spawn_blocking)
impl BitlinkCache {
    pub fn new(name: &str, cache_dir: Option<impl AsRef<Path>>) -> Option<Self> {
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

        // TODO: add a log macro
        //println!("using cache {:?}", cache_dir.join(format!("{name}.db")));

        let conn = match Connection::open(cache_dir.join(format!("{name}.db"))) {
            Ok(conn) => conn,
            Err(err) => {
                eprintln!("{APP}(cache-open): {err}");
                return None;
            }
        };

        let res = conn.execute(
            "
            CREATE TABLE IF NOT EXISTS shorten (
              id TEXT NOT NULL UNIQUE,
              link TEXT NOT NULL,
              long_url TEXT NOT NULL,
              domain TEXT,
              group_guid TEXT NOT NULL
            );

            CREATE UNIQUE INDEX IF NOT EXISTS ix_shorten
            ON shorten (group_guid, domain, long_url);
            ",
            (),
        );

        if let Err(err) = res {
            // TODO: add a log macro
            eprintln!("{APP}(cache-create): {err}");
            return None;
        }

        Some(Self { conn })
    }

    pub fn get(&self, query: &Shorten<'_>) -> Option<Bitlink> {
        let stmt = self.conn.prepare_cached(
            "
            SELECT id, link
            FROM shorten
            WHERE group_guid = :group_guid AND domain = :domain AND long_url = :long_url
            ",
        );

        let mut stmt = match stmt {
            Ok(stmt) => stmt,
            Err(err) => {
                // TODO: add a log macro
                eprintln!("{APP}(cache-get): {err}");
                return None;
            }
        };

        let params = named_params![
            ":group_guid": query.group_guid,
            ":domain": query.domain,
            ":long_url": UrlSql::from(&query.long_url),
        ];

        let res = stmt.query_row(params, |row| {
            Ok(Bitlink {
                link: row.get::<_, UrlSql>(1)?.into(),
                id: row.get(0)?,
            })
        });

        match res {
            Ok(link) => Some(link),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(error) => {
                // TODO: add a log macro
                eprintln!("{APP}(cache-get): {error}");
                None
            }
        }
    }

    pub fn set(&self, query: Shorten<'_>, link: &Bitlink) {
        let res = self.conn.execute(
            "
            INSERT INTO shorten (id, link, long_url, domain, group_guid) VALUES
            (:id, :link, :long_url, :domain, :group_guid)
            ",
            named_params![
                ":id": link.id,
                ":link": UrlSql::from(&link.link),
                ":long_url": UrlSql::from(&query.long_url),
                ":domain": query.domain,
                ":group_guid": query.group_guid,
            ],
        );

        match res {
            Ok(inserted) => debug_assert_eq!(inserted, 1),
            Err(error) => {
                // TODO: add a log macro
                eprintln!("{APP}(cache-set): {error}");
            }
        }
    }
}

#[repr(transparent)]
struct UrlSql<'a>(Cow<'a, Url>);

impl<'a> From<&'a Url> for UrlSql<'a> {
    #[inline]
    fn from(url: &'a Url) -> Self {
        Self(Cow::Borrowed(url))
    }
}

impl From<UrlSql<'_>> for Url {
    #[inline]
    fn from(UrlSql(url): UrlSql) -> Self {
        url.into_owned()
    }
}

impl FromSql for UrlSql<'_> {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        value.as_str().and_then(|url| {
            Url::try_from(url)
                .map(Cow::Owned)
                .map(Self)
                .map_err(|err| FromSqlError::Other(Box::new(err)))
        })
    }
}

impl ToSql for UrlSql<'_> {
    #[inline]
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput> {
        Ok(self.0.as_str().into())
    }
}
