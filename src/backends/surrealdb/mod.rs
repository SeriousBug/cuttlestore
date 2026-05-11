use std::{borrow::Cow, collections::HashMap};

use async_stream::try_stream;
use async_trait::async_trait;
use futures::stream::BoxStream;
use lazy_regex::regex_captures;
use serde::{Deserialize, Serialize};
use surrealdb::{
    engine::any::{connect, Any},
    opt::auth::Root,
    sql::Thing,
    Surreal,
};

use crate::{
    backend_api::{CuttleBackend, PutOptions},
    common::{get_system_time, CuttlestoreError},
};

/// Default table name used to store cuttlestore records. Can be overridden via
/// the `table=<name>` query parameter on the connection string.
const DEFAULT_TABLE: &str = "cuttlestore";

pub(crate) struct SurrealdbBackend {
    db: Surreal<Any>,
    table: String,
}

#[derive(Serialize, Deserialize)]
struct StoredRecord {
    value: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    live_until: Option<u64>,
}

#[derive(Deserialize)]
struct StoredRecordWithId {
    id: Thing,
    value: Vec<u8>,
    #[serde(default)]
    live_until: Option<u64>,
}

impl SurrealdbBackend {
    async fn open(
        endpoint: String,
        namespace: String,
        database: String,
        table: String,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<Box<Self>, CuttlestoreError> {
        let db = connect(endpoint).await?;
        if let (Some(user), Some(pass)) = (username.as_deref(), password.as_deref()) {
            db.signin(Root {
                username: user,
                password: pass,
            })
            .await?;
        }
        db.use_ns(namespace).use_db(database).await?;
        Ok(Box::new(SurrealdbBackend { db, table }))
    }
}

#[async_trait]
impl CuttleBackend for SurrealdbBackend {
    async fn new(conn: &str) -> Option<Result<Box<Self>, CuttlestoreError>> {
        // Format: surrealdb[s]://[user:pass@]host[:port]/namespace/database[?table=...]
        let (_, secure, rest) = regex_captures!(r#"^surrealdb(s)?://(.+)"#, conn)?;
        let scheme = if secure.is_empty() { "ws" } else { "wss" };

        // Pull out auth (user:pass@) if present.
        let (auth, host_and_path) = match rest.split_once('@') {
            Some((auth, host)) => (Some(auth), host),
            None => (None, rest),
        };
        let (username, password) = match auth {
            Some(auth) => match auth.split_once(':') {
                Some((u, p)) => (Some(u.to_string()), Some(p.to_string())),
                None => (Some(auth.to_string()), None),
            },
            None => (None, None),
        };

        // Split off the query string.
        let (path_part, query_part) = match host_and_path.split_once('?') {
            Some((p, q)) => (p, q),
            None => (host_and_path, ""),
        };
        let args: HashMap<&str, &str> = if query_part.is_empty() {
            HashMap::new()
        } else {
            query_part
                .split('&')
                .flat_map(|pair| pair.split_once('='))
                .collect()
        };

        // Split host from /namespace/database
        let mut segments = path_part.splitn(3, '/');
        let host = match segments.next() {
            Some(h) if !h.is_empty() => h,
            _ => {
                return Some(Err(CuttlestoreError::SurrealdbError(
                    "connection string must include host".into(),
                )))
            }
        };
        let namespace = match segments.next() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => {
                return Some(Err(CuttlestoreError::SurrealdbError(
                    "connection string must include /namespace".into(),
                )))
            }
        };
        let database = match segments.next() {
            Some(d) if !d.is_empty() => d.to_string(),
            _ => {
                return Some(Err(CuttlestoreError::SurrealdbError(
                    "connection string must include /database".into(),
                )))
            }
        };
        let table = args
            .get("table")
            .map(|t| (*t).to_string())
            .unwrap_or_else(|| DEFAULT_TABLE.to_string());

        let endpoint = format!("{scheme}://{host}/rpc");

        Some(SurrealdbBackend::open(endpoint, namespace, database, table, username, password).await)
    }

    fn requires_cleaner(&self) -> bool {
        // SurrealDB does not yet have a native per-record TTL, so we rely on
        // the external cleaner to sweep expired records.
        true
    }

    fn name(&self) -> &'static str {
        "surrealdb"
    }

    async fn get<'a>(
        &'a self,
        key: Cow<'a, str>,
    ) -> Result<Option<Cow<'a, [u8]>>, CuttlestoreError> {
        let record: Option<StoredRecord> =
            self.db.select((self.table.as_str(), key.as_ref())).await?;
        let record = match record {
            Some(r) => r,
            None => return Ok(None),
        };
        if let Some(live_until) = record.live_until {
            if live_until < get_system_time() {
                let _: Option<StoredRecord> =
                    self.db.delete((self.table.as_str(), key.as_ref())).await?;
                return Ok(None);
            }
        }
        Ok(Some(Cow::Owned(record.value)))
    }

    async fn put<'a>(
        &self,
        key: Cow<'a, str>,
        value: &[u8],
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        let live_until = options.ttl.map(|t| t + get_system_time());
        let record = StoredRecord {
            value: value.to_vec(),
            live_until,
        };
        let _: Option<StoredRecord> = self
            .db
            .upsert((self.table.as_str(), key.as_ref()))
            .content(record)
            .await?;
        Ok(())
    }

    async fn delete<'a>(&self, key: Cow<'a, str>) -> Result<(), CuttlestoreError> {
        let _: Option<StoredRecord> = self.db.delete((self.table.as_str(), key.as_ref())).await?;
        Ok(())
    }

    async fn scan<'a>(
        &'a self,
    ) -> Result<
        BoxStream<'a, Result<(String, Cow<'a, [u8]>), CuttlestoreError>>,
        CuttlestoreError,
    > {
        let records: Vec<StoredRecordWithId> = self.db.select(self.table.as_str()).await?;
        let db = self.db.clone();
        let table = self.table.clone();

        Ok(Box::pin(try_stream! {
            for record in records {
                let key = record.id.id.to_raw();
                if let Some(live_until) = record.live_until {
                    if live_until < get_system_time() {
                        // Best effort cleanup; ignore errors so a single
                        // failure does not abort the scan.
                        let _: Result<Option<StoredRecord>, _> =
                            db.delete((table.as_str(), key.as_str())).await;
                        continue;
                    }
                }
                yield (key, Cow::Owned(record.value));
            }
        }))
    }
}
