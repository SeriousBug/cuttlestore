use std::{borrow::Cow, collections::HashMap};

use async_stream::try_stream;
use async_trait::async_trait;
use futures::stream::BoxStream;
use lazy_regex::regex_captures;
use reqwest::{
    header::{ACCEPT, CONTENT_TYPE},
    Client, StatusCode, Url,
};
use serde::{Deserialize, Serialize};

use crate::{
    backend_api::{CuttleBackend, PutOptions},
    common::{get_system_time, CuttlestoreError},
};

/// The fixed name we give the single binary attachment that holds the
/// Cuttlestore value on each CouchDB document.
const ATTACHMENT_NAME: &str = "v";
/// Boundary string used for outgoing multipart/related PUT bodies. CouchDB
/// only requires that the boundary not appear in the payload, and a long
/// random-looking constant is good enough since attachment payloads are
/// arbitrary user bytes — the chance of accidental collision is negligible.
const PUT_BOUNDARY: &str = "cuttlestore-couchdb-J9Q3kpv7zxLm5sN8WrYbHcD2";

pub(crate) struct CouchdbBackend {
    client: Client,
    base: Url,
}

#[derive(Serialize, Deserialize, Default)]
struct StoredDoc {
    #[serde(rename = "_rev", default, skip_serializing_if = "Option::is_none")]
    rev: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    live_until: Option<u64>,
    #[serde(
        rename = "_attachments",
        default,
        skip_serializing_if = "HashMap::is_empty"
    )]
    attachments: HashMap<String, AttachmentMeta>,
}

#[derive(Serialize, Deserialize, Default)]
struct AttachmentMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    follows: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stub: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revpos: Option<u64>,
}

#[derive(Deserialize)]
struct AllDocsResponse {
    rows: Vec<AllDocsRow>,
}

#[derive(Deserialize)]
struct AllDocsRow {
    id: String,
}

fn doc_url(base: &Url, key: &str) -> Url {
    let mut url = base.clone();
    url.path_segments_mut()
        .expect("couchdb base url cannot be a base")
        .pop_if_empty()
        .push(key);
    url
}

/// Find the first occurrence of `needle` in `haystack` starting at `from`.
fn find_subseq(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Pull the boundary parameter out of a Content-Type header value.
fn parse_boundary(content_type: &str) -> Option<String> {
    let lower = content_type.to_ascii_lowercase();
    let idx = lower.find("boundary=")?;
    let rest = &content_type[idx + "boundary=".len()..];
    let rest = rest.trim_start();
    let (val, _) = if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        (&stripped[..end], &stripped[end + 1..])
    } else {
        let end = rest.find(';').unwrap_or(rest.len());
        (rest[..end].trim(), &rest[end..])
    };
    Some(val.to_string())
}

/// Split a multipart/related body into raw parts, returning each part's
/// header block and payload bytes. Only handles CRLF line endings, which is
/// what CouchDB emits.
fn split_multipart_parts(body: &[u8], boundary: &str) -> Result<Vec<Vec<u8>>, CuttlestoreError> {
    let delim = format!("--{boundary}");
    let delim = delim.as_bytes();
    let first = find_subseq(body, delim, 0).ok_or_else(|| {
        CuttlestoreError::CouchdbError("multipart response missing opening boundary".into())
    })?;
    let mut cursor = first + delim.len();
    let mut parts = Vec::new();

    loop {
        // Closing delimiter is `--boundary--`.
        if cursor + 2 <= body.len() && &body[cursor..cursor + 2] == b"--" {
            break;
        }
        // Each delimiter line ends with CRLF before the part starts.
        if cursor + 2 <= body.len() && &body[cursor..cursor + 2] == b"\r\n" {
            cursor += 2;
        }

        let next = find_subseq(body, delim, cursor).ok_or_else(|| {
            CuttlestoreError::CouchdbError("multipart response truncated mid-part".into())
        })?;
        // The CRLF immediately before the next delimiter belongs to the framing,
        // not to the part body.
        let mut end = next;
        if end >= 2 && &body[end - 2..end] == b"\r\n" {
            end -= 2;
        }

        let part = &body[cursor..end];
        let header_split = find_subseq(part, b"\r\n\r\n", 0).ok_or_else(|| {
            CuttlestoreError::CouchdbError("multipart part missing header terminator".into())
        })?;
        // We don't need the per-part headers — CouchDB's response always puts
        // the JSON document first followed by attachments in document order.
        let payload = part[header_split + 4..].to_vec();
        parts.push(payload);

        cursor = next + delim.len();
    }

    Ok(parts)
}

/// Build a CouchDB `multipart/related` request body for a PUT, with the
/// document JSON as the first part and the raw attachment bytes as the
/// second. CouchDB matches `follows: true` attachments to the trailing
/// parts in document attachment order.
fn build_put_body(value: &[u8], rev: Option<&str>, live_until: Option<u64>) -> Vec<u8> {
    let mut attachments = HashMap::new();
    attachments.insert(
        ATTACHMENT_NAME.to_string(),
        AttachmentMeta {
            follows: Some(true),
            content_type: Some("application/octet-stream".to_string()),
            length: Some(value.len() as u64),
            ..AttachmentMeta::default()
        },
    );
    let doc = StoredDoc {
        rev: rev.map(|s| s.to_string()),
        live_until,
        attachments,
    };
    let json_bytes = serde_json::to_vec(&doc).expect("StoredDoc always serializes cleanly");

    let mut body = Vec::with_capacity(value.len() + json_bytes.len() + 256);
    body.extend_from_slice(format!("--{PUT_BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/json\r\n\r\n");
    body.extend_from_slice(&json_bytes);
    body.extend_from_slice(format!("\r\n--{PUT_BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(value);
    body.extend_from_slice(format!("\r\n--{PUT_BOUNDARY}--\r\n").as_bytes());
    body
}

impl CouchdbBackend {
    async fn open(url: Url) -> Result<Box<Self>, CuttlestoreError> {
        let client = Client::builder().build()?;
        // Ensure the database exists. CouchDB returns 201 on create and 412 if
        // it already exists; either is acceptable. We deliberately ignore
        // other status codes (e.g. 401) here — a missing DB will surface
        // clearly on the first real operation.
        let _ = client.put(url.clone()).send().await;
        Ok(Box::new(CouchdbBackend { client, base: url }))
    }

    /// Fetch just the document metadata (no attachment payload). Used for
    /// looking up `_rev` before deletes and conflicting puts.
    async fn get_metadata(&self, key: &str) -> Result<Option<StoredDoc>, CuttlestoreError> {
        let resp = self.client.get(doc_url(&self.base, key)).send().await?;
        match resp.status() {
            StatusCode::OK => Ok(Some(resp.json().await?)),
            StatusCode::NOT_FOUND => Ok(None),
            s => Err(CuttlestoreError::CouchdbError(format!(
                "GET {key} returned {s}"
            ))),
        }
    }

    async fn get_with_attachment(
        &self,
        key: &str,
    ) -> Result<Option<(StoredDoc, Vec<u8>)>, CuttlestoreError> {
        let mut url = doc_url(&self.base, key);
        url.query_pairs_mut().append_pair("attachments", "true");
        let resp = self
            .client
            .get(url)
            .header(ACCEPT, "multipart/related")
            .send()
            .await?;
        match resp.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Ok(None),
            s => {
                return Err(CuttlestoreError::CouchdbError(format!(
                    "GET {key} returned {s}"
                )))
            }
        }

        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp.bytes().await?;

        // CouchDB returns plain JSON when there are no attachments to deliver.
        // We always store one, so this path mostly catches edge cases.
        if content_type.starts_with("application/json") {
            let doc: StoredDoc = serde_json::from_slice(&body)?;
            return Ok(Some((doc, Vec::new())));
        }

        let boundary = parse_boundary(&content_type).ok_or_else(|| {
            CuttlestoreError::CouchdbError(format!(
                "missing boundary in Content-Type: {content_type}"
            ))
        })?;
        let parts = split_multipart_parts(&body, &boundary)?;
        let mut iter = parts.into_iter();
        let json_bytes = iter.next().ok_or_else(|| {
            CuttlestoreError::CouchdbError("multipart response had no parts".into())
        })?;
        let doc: StoredDoc = serde_json::from_slice(&json_bytes)?;
        // The first attachment part holds the value bytes. There should only
        // ever be one because we only attach `ATTACHMENT_NAME`.
        let value = iter.next().unwrap_or_default();
        Ok(Some((doc, value)))
    }

    async fn put_doc(
        &self,
        key: &str,
        value: &[u8],
        live_until: Option<u64>,
    ) -> Result<(), CuttlestoreError> {
        let mut rev: Option<String> = None;
        // Fast path: try without rev. On conflict, pick up the existing rev
        // and retry once. This avoids a round trip on the common case of
        // creating new keys.
        for attempt in 0..2 {
            let body = build_put_body(value, rev.as_deref(), live_until);
            let resp = self
                .client
                .put(doc_url(&self.base, key))
                .header(
                    CONTENT_TYPE,
                    format!("multipart/related; boundary=\"{PUT_BOUNDARY}\""),
                )
                .body(body)
                .send()
                .await?;
            match resp.status() {
                s if s.is_success() => return Ok(()),
                StatusCode::CONFLICT if attempt == 0 => {
                    let existing = self.get_metadata(key).await?;
                    rev = existing.and_then(|d| d.rev);
                    if rev.is_none() {
                        return Err(CuttlestoreError::CouchdbError(
                            "PUT conflict but no existing document found".into(),
                        ));
                    }
                }
                s => {
                    return Err(CuttlestoreError::CouchdbError(format!(
                        "PUT {key} returned {s}"
                    )))
                }
            }
        }
        Err(CuttlestoreError::CouchdbError(
            "PUT exceeded retry attempts".into(),
        ))
    }

    async fn delete_doc(&self, key: &str) -> Result<(), CuttlestoreError> {
        let rev = match self.get_metadata(key).await? {
            Some(doc) => doc.rev,
            None => return Ok(()),
        };
        let rev = match rev {
            Some(rev) => rev,
            None => return Ok(()),
        };
        let mut url = doc_url(&self.base, key);
        url.query_pairs_mut().append_pair("rev", &rev);
        let resp = self.client.delete(url).send().await?;
        match resp.status() {
            s if s.is_success() => Ok(()),
            // If something else updated the doc between our GET and DELETE,
            // treat as already gone — the caller asked us to delete it and
            // there is nothing under that key anymore from their perspective.
            StatusCode::NOT_FOUND | StatusCode::CONFLICT => Ok(()),
            s => Err(CuttlestoreError::CouchdbError(format!(
                "DELETE {key} returned {s}"
            ))),
        }
    }
}

#[async_trait]
impl CuttleBackend for CouchdbBackend {
    async fn new(conn: &str) -> Option<Result<Box<Self>, CuttlestoreError>> {
        let (_, secure, rest) = regex_captures!(r#"^couchdb(s)?://(.+)"#, conn)?;
        let scheme = if secure.is_empty() { "http" } else { "https" };
        let url_str = format!("{scheme}://{rest}");
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
                "connection string must include a database name".into(),
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

    async fn get<'a>(
        &'a self,
        key: Cow<'a, str>,
    ) -> Result<Option<Cow<'a, [u8]>>, CuttlestoreError> {
        let (doc, value) = match self.get_with_attachment(key.as_ref()).await? {
            Some(pair) => pair,
            None => return Ok(None),
        };
        if let Some(live_until) = doc.live_until {
            if live_until < get_system_time() {
                self.delete_doc(key.as_ref()).await?;
                return Ok(None);
            }
        }
        Ok(Some(Cow::Owned(value)))
    }

    async fn put<'a>(
        &self,
        key: Cow<'a, str>,
        value: &[u8],
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        let live_until = options.ttl.map(|t| t + get_system_time());
        self.put_doc(key.as_ref(), value, live_until).await
    }

    async fn delete<'a>(&self, key: Cow<'a, str>) -> Result<(), CuttlestoreError> {
        self.delete_doc(key.as_ref()).await
    }

    async fn scan<'a>(
        &'a self,
    ) -> Result<
        BoxStream<'a, Result<(String, Cow<'a, [u8]>), CuttlestoreError>>,
        CuttlestoreError,
    > {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("couchdb base url cannot be a base")
            .pop_if_empty()
            .push("_all_docs");

        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(CuttlestoreError::CouchdbError(format!(
                "_all_docs returned {}",
                resp.status()
            )));
        }
        let body: AllDocsResponse = resp.json().await?;

        // Clone what the stream needs so it does not borrow `self`.
        let client = self.client.clone();
        let base = self.base.clone();

        Ok(Box::pin(try_stream! {
            for row in body.rows {
                // CouchDB's _all_docs lists design and local documents alongside
                // user data. They start with `_` so we skip them outright.
                if row.id.starts_with('_') {
                    continue;
                }
                let backend = CouchdbBackend { client: client.clone(), base: base.clone() };
                let (doc, value) = match backend.get_with_attachment(&row.id).await? {
                    Some(pair) => pair,
                    // Doc was deleted between _all_docs and the GET — skip it.
                    None => continue,
                };
                if let Some(live_until) = doc.live_until {
                    if live_until < get_system_time() {
                        // Best effort: try to delete the expired document. We
                        // ignore errors here so a single failing delete does
                        // not abort the entire scan.
                        let _ = backend.delete_doc(&row.id).await;
                        continue;
                    }
                }
                yield (row.id, Cow::Owned(value));
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_boundary_quoted() {
        assert_eq!(
            parse_boundary("multipart/related; boundary=\"abc\""),
            Some("abc".to_string())
        );
    }

    #[test]
    fn parse_boundary_unquoted() {
        assert_eq!(
            parse_boundary("multipart/related; boundary=abc; charset=utf8"),
            Some("abc".to_string())
        );
    }

    #[test]
    fn split_multipart_round_trip() {
        let body = b"--B\r\n\
Content-Type: application/json\r\n\
\r\n\
{\"a\":1}\r\n\
--B\r\n\
Content-Type: application/octet-stream\r\n\
\r\n\
\x00\x01\x02hello\xffworld\r\n\
--B--\r\n";
        let parts = split_multipart_parts(body, "B").unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(&parts[0], b"{\"a\":1}");
        assert_eq!(&parts[1], b"\x00\x01\x02hello\xffworld");
    }
}
