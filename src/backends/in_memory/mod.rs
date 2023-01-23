use std::borrow::Cow;

use async_trait::async_trait;
use dashmap::DashMap;
use futures::stream::BoxStream;
use lazy_regex::regex_is_match;

use crate::{
    backend_api::{CuttleBackend, PutOptions},
    common::{get_system_time, CuttlestoreError},
};

struct StoredValue {
    payload: Vec<u8>,
    live_until: Option<u64>,
}

pub(crate) struct InMemoryBackend {
    map: DashMap<String, StoredValue>,
}

impl InMemoryBackend {
    async fn delete(&self, key: &str) {
        self.map.remove(key);
    }

    async fn get(&self, key: &str) -> Option<Vec<u8>> {
        match self.map.get(key) {
            Some(value) => {
                if let Some(live_until) = value.live_until {
                    if live_until < get_system_time() {
                        drop(value); // Need to drop before deleting or it may deadlock
                        self.delete(key).await;
                        return None;
                    }
                }
                Some(value.payload.clone())
            }
            None => None,
        }
    }
}

#[async_trait]
impl CuttleBackend for InMemoryBackend {
    async fn new(conn: &str) -> Option<Result<Box<Self>, CuttlestoreError>> {
        if regex_is_match!(r#"^in-memory$"#, conn) {
            Some(Ok(Box::new(InMemoryBackend {
                map: DashMap::new(),
            })))
        } else {
            None
        }
    }

    fn requires_cleaner(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "in-memory"
    }

    async fn get<'a>(&self, key: Cow<'a, str>) -> Result<Option<Vec<u8>>, CuttlestoreError> {
        Ok(self.get(key.as_ref()).await)
    }

    async fn put<'a>(
        &self,
        key: Cow<'a, str>,
        value: &[u8],
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        self.map.insert(
            key.to_string(),
            StoredValue {
                payload: value.to_vec(),
                live_until: options.ttl.map(|t| get_system_time() + t),
            },
        );
        Ok(())
    }

    async fn delete<'a>(&self, key: Cow<'a, str>) -> Result<(), CuttlestoreError> {
        self.delete(key.as_ref()).await;
        Ok(())
    }

    async fn scan(
        &self,
    ) -> Result<BoxStream<Result<(String, Vec<u8>), CuttlestoreError>>, CuttlestoreError> {
        Ok(Box::pin(futures::stream::iter(
            self.map
                .iter()
                .filter(|v| {
                    // Filter out expired pairs
                    if let Some(live_until) = v.live_until {
                        return live_until > get_system_time();
                    }
                    true
                })
                // Copy out the data
                .map(|v| Ok((v.key().to_string(), v.payload.clone()))),
        )))
    }
}
