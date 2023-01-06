use std::{marker::PhantomData, sync::Arc, time::Duration};

use async_stream::try_stream;
use futures::stream::BoxStream;
use lazy_regex::regex_captures;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    backend_api::{CuttleBackend, PutOptions},
    common::{
        cleanup::{Cleaner, CleanerOptions},
        CuttlestoreError,
    },
};

#[derive(Clone)]
/// A basic key-value store.
///
/// The key-value store is associated with a single type that can be serialized
/// and deserialized using Serde (`Serialize` + `DeserializeOwned`). The type
/// also needs to be safe to send to other threads (`Send` + `Sync`).
///
/// The Cuttlestore can be safely cloned, all the clones will access the same
/// store and will not use additional resources.
///
/// With some backends (filesystem, in-memory), Cuttlestore will need to
/// periodically scan the store to clear out expired key-value pairs. This could
/// be a performance issue if you have too many pairs: you should pick a backend
/// that does not require scans or disable the scans if this is a problem.
pub struct Cuttlestore<Value: Serialize + DeserializeOwned + Send + Sync> {
    /// The actual store backend.
    store: Arc<Box<dyn CuttleBackend + Send + Sync>>,
    #[allow(dead_code)]
    /// For backends that require it, a cleaner is created which will scan the
    /// store periodically to drop expired entries.
    ///
    /// While the cleaner property is not used directly, we need to keep the
    /// cleaner around because it will stop when dropped.
    cleaner: Option<Arc<Cleaner>>,

    /// A placeholder to hide that Value is not used within the struct. While
    /// unused, Value is part of the Cuttlestore type to ensure users don't
    /// accidentally put and get different types from the same store, which
    /// would fail.
    phantom: std::marker::PhantomData<Value>,
}

impl<Value: Serialize + DeserializeOwned + Send + Sync> std::fmt::Debug for Cuttlestore<Value> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cuttlestore")
            .field("backend", &self.store.name())
            .finish()
    }
}

impl<Value: Serialize + DeserializeOwned + Send + Sync> Cuttlestore<Value> {
    /// Create a connection to the store.
    ///
    /// You connect to the store using a connection string. A few examples are:
    ///
    /// - Cuttlestore::new("redis://127.0.0.1")
    /// - Cuttlestore::new("filesystem://./in-working-folder")
    /// - Cuttlestore::new("filesystem:///in-root-folder")
    /// - Cuttlestore::new("sqlite://path/to/db/file")
    /// - Cuttlestore::new("in-memory")
    ///
    /// The selection of the store happens at runtime, so you can use a
    /// user-provided string to select the store.
    ///
    /// Stores are only available if the corresponding feature is enabled. They
    /// all are by default, but mind that if you disable the default features.
    pub async fn new<C: AsRef<str>>(conn: C) -> Result<Self, CuttlestoreError> {
        let store = Arc::new(find_matching_backend(conn.as_ref()).await?);
        let cleaner: Option<Arc<Cleaner>> = if store.requires_cleaner() {
            Some(Arc::new(Cleaner::new(
                store.clone(),
                CleanerOptions {
                    sweep_every: Duration::from_secs(60),
                },
            )))
        } else {
            None
        };
        Ok(Cuttlestore {
            store,
            cleaner,
            phantom: PhantomData,
        })
    }

    pub async fn put<Key: AsRef<str>>(
        &self,
        key: Key,
        value: &Value,
    ) -> Result<(), CuttlestoreError> {
        self.put_with(key, value, PutOptions::default()).await
    }

    pub async fn put_with<Key: AsRef<str>>(
        &self,
        key: Key,
        value: &Value,
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        let payload = bincode::serialize(value)?;
        self.store.put(key.as_ref(), &payload[..], options).await
    }

    pub async fn delete<Key: AsRef<str>>(&self, key: Key) -> Result<(), CuttlestoreError> {
        self.store.delete(key.as_ref()).await
    }

    pub async fn get<Key: AsRef<str>>(&self, key: Key) -> Result<Option<Value>, CuttlestoreError> {
        let payload = self.store.get(key.as_ref()).await?;
        let value = payload
            .map(|payload| {
                let value: Value = bincode::deserialize(&payload[..])?;
                Ok::<Value, CuttlestoreError>(value)
            })
            .transpose()?;
        Ok(value)
    }

    pub async fn scan(
        &self,
    ) -> Result<BoxStream<Result<(String, Value), CuttlestoreError>>, CuttlestoreError> {
        let stream = self.store.scan().await?;

        Ok(Box::pin(try_stream! {
            for await pair in stream {
                let (key, payload) = pair?;
                let value: Value = bincode::deserialize(&payload[..])?;
                yield (key, value);
            }
        }))
    }
}

async fn find_matching_backend(
    conn: &str,
) -> Result<Box<dyn CuttleBackend + Send + Sync>, CuttlestoreError> {
    #[cfg(feature = "backend-filesystem")]
    if let Some(backend) = crate::backends::filesystem::FilesystemBackend::new(conn).await {
        return Ok(backend?);
    }
    #[cfg(feature = "backend-in-memory")]
    if let Some(backend) = crate::backends::in_memory::InMemoryBackend::new(conn).await {
        return Ok(backend?);
    }
    #[cfg(feature = "backend-redis")]
    if let Some(backend) = crate::backends::redis::RedisBackend::new(conn).await {
        return Ok(backend?);
    }
    #[cfg(feature = "backend-sqlite")]
    if let Some(backend) = crate::backends::sqlite::SqliteBackend::new(conn).await {
        return Ok(backend?);
    }

    let maybe_backend = regex_captures!(r#"^([^:]+):"#, conn);
    Err(CuttlestoreError::NoMatchingBackend(
        maybe_backend
            .map(|(_, name)| name)
            .unwrap_or_else(|| conn)
            .to_string(),
    ))
}
