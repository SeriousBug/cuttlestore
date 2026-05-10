use std::borrow::Cow;

use async_stream::try_stream;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use futures::stream::BoxStream;
use lazy_regex::regex_captures;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};

use crate::{
    backend_api::{CuttleBackend, PutOptions},
    common::{get_system_time, CuttlestoreError},
};

pub(crate) struct CouchdbBackend {
    client: Client,
    base: Url,
}

#[derive(Serialize, Deserialize, Clone)]
struct StoredDoc {
    #[serde(rename = "_rev", skip_serializing_if = "Option::is_none")]
    rev: Option<String>,
    value: String,
    live_until: Option<u64>,
}

#[derive(Deserialize)]
struct AllDocsResponse {
    rows: Vec<AllDocsRow>,
}

#[derive(Deserialize)]
struct AllDocsRow {
    id: String,
    doc: Option<StoredDoc>,
}

fn doc_url(base: &Url, key: &str) -> Url {
    let mut url = base.clone();
    url.path_segments_mut()
        .expect("couchdb base url cannot be a base")
        .pop_if_empty()
        .push(key);
    url
}

impl CouchdbBackend {
    async fn open(url: Url) -> Result<Box<Self>, CuttlestoreError> {
        let client = Client::builder().build()?;
        // Ensure database exists. CouchDB returns 201 on create, 412 if it already exists.
        // Other status codes (e.g. 401) are ignored here; a missing db will surface on first op.
        let _ = client.put(url.clone()).send().await;

        Ok(Box::new(CouchdbBackend { client, base: url }))
    }

    async fn get_doc(&self, key: &str) -> Result<Option<StoredDoc>, CuttlestoreError> {
        let resp = self.client.get(doc_url(&self.base, key)).send().await?;
        match resp.status() {
            StatusCode::OK => Ok(Some(resp.json().await?)),
            StatusCode::NOT_FOUND => Ok(None),
            s => Err(CuttlestoreError::CouchdbError(format!(
                "GET {} returned {}",
                key, s
            ))),
        }
    }

    async fn put_doc(&self, key: &str, doc: &StoredDoc) -> Result<(), CuttlestoreError> {
        let mut doc = doc.clone();
        // Fast path: try without rev, then fall back to fetching the current rev on conflict.
        for attempt in 0..2 {
            let resp = self
                .client
                .put(doc_url(&self.base, key))
                .json(&doc)
                .send()
                .await?;
            match resp.status() {
                s if s.is_success() => return Ok(()),
                StatusCode::CONFLICT if attempt == 0 => {
                    let existing = self.get_doc(key).await?;
                    doc.rev = existing.and_then(|d| d.rev);
                    if doc.rev.is_none() {
                        return Err(CuttlestoreError::CouchdbError(
                            "PUT conflict but no existing document found".to_string(),
                        ));
                    }
                }
                s => {
                    return Err(CuttlestoreError::CouchdbError(format!(
                        "PUT {} returned {}",
                        key, s
                    )))
                }
            }
        }
        Err(CuttlestoreError::CouchdbError(
            "PUT exceeded retry attempts".to_string(),
        ))
    }

    async fn delete_doc(&self, key: &str) -> Result<(), CuttlestoreError> {
        let existing = match self.get_doc(key).await? {
            Some(doc) => doc,
            None => return Ok(()),
        };
        let rev = match existing.rev {
            Some(rev) => rev,
            None => return Ok(()),
        };
        let mut url = doc_url(&self.base, key);
        url.query_pairs_mut().append_pair("rev", &rev);
        let resp = self.client.delete(url).send().await?;
        match resp.status() {
            s if s.is_success() => Ok(()),
            StatusCode::NOT_FOUND => Ok(()),
            // If something else updated the doc between our GET and DELETE, treat as already gone.
            StatusCode::CONFLICT => Ok(()),
            s => Err(CuttlestoreError::CouchdbError(format!(
                "DELETE {} returned {}",
                key, s
            ))),
        }
    }
}

#[async_trait]
impl CuttleBackend for CouchdbBackend {
    async fn new(conn: &str) -> Option<Result<Box<Self>, CuttlestoreError>> {
        let (_, secure, rest) = regex_captures!(r#"^couchdb(s)?://(.+)"#, conn)?;
        let scheme = if secure.is_empty() { "http" } else { "https" };
        let url_str = format!("{}://{}", scheme, rest);
        let url = match Url::parse(&url_str) {
            Ok(u) => u,
            Err(e) => {
                return Some(Err(CuttlestoreError::CouchdbError(format!(
                    "invalid url: {e}"
                ))))
            }
        };
        if url.path().is_empty() || url.path() == "/" {
            return Some(Err(CuttlestoreError::CouchdbError(
                "connection string must include a database name".to_string(),
            )));
        }
        Some(CouchdbBackend::open(url).await)
    }

    fn requires_cleaner(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "couchdb"
    }

    async fn get<'a>(&self, key: Cow<'a, str>) -> Result<Option<Vec<u8>>, CuttlestoreError> {
        let doc = match self.get_doc(key.as_ref()).await? {
            Some(doc) => doc,
            None => return Ok(None),
        };
        if let Some(live_until) = doc.live_until {
            if live_until < get_system_time() {
                self.delete_doc(key.as_ref()).await?;
                return Ok(None);
            }
        }
        let bytes = B64.decode(doc.value.as_bytes())?;
        Ok(Some(bytes))
    }

    async fn put<'a>(
        &self,
        key: Cow<'a, str>,
        value: &[u8],
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        let live_until = options.ttl.map(|t| t + get_system_time());
        let doc = StoredDoc {
            rev: None,
            value: B64.encode(value),
            live_until,
        };
        self.put_doc(key.as_ref(), &doc).await
    }

    async fn delete<'a>(&self, key: Cow<'a, str>) -> Result<(), CuttlestoreError> {
        self.delete_doc(key.as_ref()).await
    }

    async fn scan(
        &self,
    ) -> Result<BoxStream<Result<(String, Vec<u8>), CuttlestoreError>>, CuttlestoreError> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("couchdb base url cannot be a base")
            .pop_if_empty()
            .push("_all_docs");
        url.query_pairs_mut().append_pair("include_docs", "true");

        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(CuttlestoreError::CouchdbError(format!(
                "_all_docs returned {}",
                resp.status()
            )));
        }
        let body: AllDocsResponse = resp.json().await?;
        let client = self.client.clone();
        let base = self.base.clone();

        Ok(Box::pin(try_stream! {
            for row in body.rows {
                // CouchDB's _all_docs includes design documents, which start with "_design/".
                // Skip those entirely so we never expose internal docs to users.
                if row.id.starts_with('_') {
                    continue;
                }
                let doc = match row.doc {
                    Some(doc) => doc,
                    None => continue,
                };
                if let Some(live_until) = doc.live_until {
                    if live_until < get_system_time() {
                        // Best-effort delete; ignore errors during scan-cleanup.
                        if let Some(rev) = doc.rev {
                            let mut url = doc_url(&base, &row.id);
                            url.query_pairs_mut().append_pair("rev", &rev);
                            let _ = client.delete(url).send().await;
                        }
                        continue;
                    }
                }
                let bytes = B64.decode(doc.value.as_bytes())?;
                yield (row.id, bytes);
            }
        }))
    }
}
