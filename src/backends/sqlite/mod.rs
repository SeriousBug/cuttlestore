use async_stream::try_stream;
use async_trait::async_trait;
use futures::stream::BoxStream;
use lazy_regex::regex_captures;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

use crate::{
    backend_api::{CuttleBackend, PutOptions},
    common::{get_system_time, CuttlestoreError},
};

pub(crate) struct SqliteBackend {
    pool: SqlitePool,
}

impl SqliteBackend {
    async fn new(path: &str) -> Result<Box<Self>, CuttlestoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .statement_cache_capacity(10)
            // Use write-ahead logging journal, with a less strict sync
            // mode. This presents a small risk of data loss, but no risk of
            // corruption.
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            // shared cache with read_uncommitted drops the isolation level for better performance
            .shared_cache(true)
            .pragma("read_uncommitted", "true");
        let pool = SqlitePool::connect_with(options).await?;
        // Create the table in case it is missing
        sqlx::query("CREATE TABLE IF NOT EXISTS cuttlestore (key STRING PRIMARY KEY NOT NULL, value BLOB NOT NULL, live_until INTEGER)").execute(&pool).await?;
        Ok(Box::new(SqliteBackend { pool }))
    }
}

#[async_trait]
impl CuttleBackend for SqliteBackend {
    async fn new(conn: &str) -> Option<Result<Box<Self>, CuttlestoreError>> {
        if let Some((_, path)) = regex_captures!(r#"^sqlite://(.+)"#, conn) {
            Some(SqliteBackend::new(path).await)
        } else {
            None
        }
    }

    fn requires_cleaner(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "sqlite"
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CuttlestoreError> {
        let row: Option<(Vec<u8>, Option<i64>)> = sqlx::query_as(
            r#"SELECT value, live_until as "live_until?" FROM cuttlestore WHERE key = ?"#,
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some((value, live_until)) => {
                if let Some(live_until) = live_until {
                    if live_until < get_system_time() as i64 {
                        self.delete(key).await?;
                        return Ok(None);
                    }
                }
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    async fn put(
        &self,
        key: &str,
        value: &[u8],
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        // Oops, sqlite can't store i64's. Casting should be fine though for
        let live_until = options.ttl.map(|t| (t + get_system_time()) as i64);
        sqlx::query("INSERT INTO cuttlestore (key, value, live_until) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = ?, live_until = ?")
            .bind(key)
            .bind(value)
            .bind(live_until)
            .bind(value)
            .bind(live_until)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), CuttlestoreError> {
        sqlx::query("DELETE FROM cuttlestore WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn scan(
        &self,
    ) -> Result<BoxStream<Result<(String, Vec<u8>), CuttlestoreError>>, CuttlestoreError> {
        let rows = sqlx::query_as::<_, (String, Vec<u8>, Option<i64>)>(
            "SELECT key, value, live_until FROM cuttlestore",
        )
        .fetch(&self.pool);

        Ok(Box::pin(try_stream! {
          for await row in rows {
            let (key, value, live_until) = row?;

            if let Some(live_until) = live_until {
                if live_until < get_system_time() as i64 {
                    self.delete(&key).await?;
                    continue;
                  }
            }

            yield (key, value);
          }
        }))
    }
}
