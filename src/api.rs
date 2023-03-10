use std::{borrow::Cow, marker::PhantomData, sync::Arc};

use async_stream::try_stream;
use futures::stream::BoxStream;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    backend_api::{CuttleBackend, PutOptions},
    builder::find_matching_backend,
    common::{
        cleanup::{Cleaner, CleanerOptions},
        CuttlestoreError,
    },
};

#[derive(Clone)]
/// A basic key-value store. This is the primary API you'll interact with.
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
    pub(crate) store: Arc<Box<dyn CuttleBackend + Send + Sync>>,
    #[allow(dead_code)]
    /// For backends that require it, a cleaner is created which will scan the
    /// store periodically to drop expired entries.
    ///
    /// While the cleaner property is not used directly, we need to keep the
    /// cleaner around because it will stop when dropped.
    pub(crate) cleaner: Option<Arc<Cleaner>>,

    /// A placeholder to hide that Value is not used within the struct. While
    /// unused, Value is part of the Cuttlestore type to ensure users don't
    /// accidentally put and get different types from the same store, which
    /// would fail.
    pub(crate) phantom: std::marker::PhantomData<Value>,

    /// If exists, the keys for all operations on this store will be prefix with
    /// this value .
    pub(crate) prefix: Option<String>,
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
    /// ```
    /// use cuttlestore::Cuttlestore;
    ///
    /// # tokio_test::block_on(async {
    /// let store: Cuttlestore<String> = Cuttlestore::new("redis://127.0.0.1").await.unwrap();
    /// //                     or any other type with serde!
    /// let store: Cuttlestore<String> = Cuttlestore::new("filesystem://./example-store/relative-path").await.unwrap();
    /// let store: Cuttlestore<String> = Cuttlestore::new("filesystem:///tmp/absolute-path").await.unwrap();
    /// let store: Cuttlestore<String> = Cuttlestore::new("sqlite://./example-store/some-path").await.unwrap();
    /// let store: Cuttlestore<String> = Cuttlestore::new("in-memory").await.unwrap();
    /// # })
    /// ```
    ///
    /// The selection of the store happens at runtime, so you can use a
    /// user-provided string to select the store.
    ///
    /// Stores are only available if the corresponding feature is enabled. They
    /// all are by default, but mind that if you disable the default features.
    ///
    /// This will open the store with the default settings. You can configure
    /// settings or open multiple stores that share the same connection using
    /// the builder API, please check [CuttlestoreBuilder](crate::CuttlestoreBuilder).
    pub async fn new<C: AsRef<str>>(conn: C) -> Result<Self, CuttlestoreError> {
        Self::make(conn.as_ref(), CleanerOptions::default()).await
    }

    pub(crate) async fn make(
        conn: &str,
        cleaner_options: CleanerOptions,
    ) -> Result<Self, CuttlestoreError> {
        let store = Arc::new(find_matching_backend(conn).await?);
        let cleaner: Option<Arc<Cleaner>> = if store.requires_cleaner() {
            Some(Arc::new(Cleaner::new(store.clone(), cleaner_options)))
        } else {
            None
        };
        Ok(Cuttlestore {
            store,
            cleaner,
            prefix: None,
            phantom: PhantomData,
        })
    }

    /// Prefixes the key, if one is configured for this store.
    fn key<'a>(&self, key: &'a str) -> Cow<'a, str> {
        match &self.prefix {
            Some(prefix) => Cow::Owned(format!("{prefix}:{key}")),
            None => Cow::Borrowed(key),
        }
    }

    /// Strip a prefix from the key, if one is configured for this store.
    fn strip_prefix(&self, prefixed_key: String) -> Option<String> {
        match &self.prefix {
            Some(prefix) => match prefixed_key
                // strip the prefix
                .strip_prefix(prefix)
                // then strip the : that connects the prefix
                .map(|key| key.strip_prefix(':'))
            {
                Some(Some(key)) => Some(key.to_string()),
                _ => None,
            },
            None => Some(prefixed_key),
        }
    }

    /// Place a value into the store with the default settings.
    pub async fn put<Key: AsRef<str>>(
        &self,
        key: Key,
        value: &Value,
    ) -> Result<(), CuttlestoreError> {
        self.put_with(key, value, PutOptions::default()).await
    }

    /// Place a value into the store, configuring the settings for this operation.
    pub async fn put_with<Key: AsRef<str>>(
        &self,
        key: Key,
        value: &Value,
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        let payload = bincode::serialize(value)?;
        self.store
            .put(self.key(key.as_ref()), &payload[..], options)
            .await
    }

    /// Remove a value from the store.
    pub async fn delete<Key: AsRef<str>>(&self, key: Key) -> Result<(), CuttlestoreError> {
        self.store.delete(self.key(key.as_ref())).await
    }

    /// Get a value from the store.
    ///
    /// This operation is guaranteed to never return expired values.
    pub async fn get<Key: AsRef<str>>(&self, key: Key) -> Result<Option<Value>, CuttlestoreError> {
        let payload = self.store.get(self.key(key.as_ref())).await?;
        let value = payload
            .map(|payload| {
                let value: Value = bincode::deserialize(&payload[..])?;
                Ok::<Value, CuttlestoreError>(value)
            })
            .transpose()?;
        Ok(value)
    }

    /// Get a stream of all the key and value pairs in the store.
    ///
    /// This operation is guaranteed to never return expired values.
    ///
    /// This is a very inefficient operation as it has to iterate over all the
    /// values in the store. (for now) using multiple stores connected to the
    /// same backing storage won't improve the performance either, the scan will
    /// iterate over values of all stores and discard ones for other stores.
    pub async fn scan(
        &self,
    ) -> Result<BoxStream<Result<(String, Value), CuttlestoreError>>, CuttlestoreError> {
        let stream = self.store.scan().await?;

        Ok(Box::pin(try_stream! {
            for await pair in stream {
                let (key, payload) = pair?;
                if let Some(key) = self.strip_prefix(key) {
                    let value: Value = bincode::deserialize(&payload[..])?;

                    yield (key, value);
                }
            }
        }))
    }
}
