use std::{marker::PhantomData, sync::Arc, time::Duration};

use lazy_regex::regex_captures;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    backend_api::CuttleBackend,
    common::{
        cleanup::{Cleaner, CleanerOptions},
        CuttlestoreError,
    },
    Cuttlestore,
};

#[derive(Debug)]
/// Configure a Cuttlestore.
pub struct CuttlestoreBuilder {
    conn: String,
    cleaner: CleanerOptions,
    prefix: Option<String>,
}

impl CuttlestoreBuilder {
    /// Start configuring your Cuttlestore.
    pub fn new<C: AsRef<str>>(conn: C) -> Self {
        Self {
            conn: conn.as_ref().to_string(),
            cleaner: CleanerOptions::default(),
            prefix: None,
        }
    }

    /// Every period, scan the store for stale values.
    ///
    /// This only works for backends that do not have built-in TTL support.
    /// Currently this is all backends except Redis. For Redis, you'll need to
    /// look at Redis configuration if you need to change how often stale data
    /// is cleaned up.
    ///
    /// For other backends, this will do nothing.
    pub fn clean_every(mut self, period: Duration) -> Self {
        self.cleaner.sweep_every = period;
        self
    }

    /// Every this many seconds, scan the store for stale values.
    ///
    /// This only works for backends that do not have built-in TTL support.
    /// Currently this is all backends except Redis. For Redis, you'll need to
    /// look at Redis configuration if you need to change how often stale data
    /// is cleaned up.
    ///
    /// For other backends, this will do nothing.
    pub fn clean_every_secs(self, secs: u64) -> Self {
        self.clean_every(Duration::from_secs(secs))
    }

    /// Prefix every key in the store with this string.
    ///
    /// The prefix is entirely transparent to your application. It is
    /// transparently applied during operations, and stripped when scanning the
    /// store. You don't need to add the prefix yourself when calling `get` or
    /// `put`.
    ///
    /// If you need multiple applications to share the same backing store,
    /// adding a unique prefix can help you avoid conflicts.
    pub fn prefix<C: AsRef<str>>(mut self, prefix: C) -> Self {
        self.prefix = Some(prefix.as_ref().to_owned());
        self
    }

    /// Finish configuring your Cuttlestore, finalizing it so you can use it.
    pub async fn finish<Value: Serialize + DeserializeOwned + Send + Sync>(
        self,
    ) -> Result<Cuttlestore<Value>, CuttlestoreError> {
        Cuttlestore::make(&self.conn, self.cleaner).await
    }

    /// Finish configuring your Cuttlestore, opening it as a CuttleConnection so
    /// you can make multiple Cuttlestores.
    ///
    /// If you need to store multiple types of objects within the same backend,
    /// you should use this function to create a connection. You can then use
    /// the connection to create multiple Cuttlestores.
    pub async fn finish_connection(self) -> Result<CuttleConnection, CuttlestoreError> {
        CuttleConnection::new(self).await
    }
}

/// A connection to the backing data store. You can use this connection to
/// create one or more Cuttlestore instances, which will share their
/// connections.
pub struct CuttleConnection {
    /// The actual store backend.
    store: Arc<Box<dyn CuttleBackend + Send + Sync>>,
    #[allow(dead_code)]
    /// For backends that require it, a cleaner is created which will scan the
    /// store periodically to drop expired entries.
    ///
    /// While the cleaner property is not used directly, we need to keep the
    /// cleaner around because it will stop when dropped.
    cleaner: Option<Arc<Cleaner>>,
    /// Prefix for all stores made out of this connection.
    prefix: Option<String>,
}

impl CuttleConnection {
    pub(crate) async fn new(builder: CuttlestoreBuilder) -> Result<Self, CuttlestoreError> {
        let store = Arc::new(find_matching_backend(&builder.conn).await?);
        let cleaner: Option<Arc<Cleaner>> = if store.requires_cleaner() {
            Some(Arc::new(Cleaner::new(store.clone(), builder.cleaner)))
        } else {
            None
        };
        Ok(Self {
            store,
            cleaner,
            prefix: builder.prefix,
        })
    }
}

impl std::fmt::Debug for CuttleConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CuttlestoreConnection")
            .field("backend", &self.store.name())
            .finish()
    }
}

impl CuttleConnection {
    /// Create a new Cuttlestore using this connection. All Cuttlestores created
    /// on the same connection share the same underlying database connections.
    ///
    /// You must pick a unique prefix for each store you create. Otherwise keys
    /// from different Cuttlestores will conflict with each other, and you will
    /// get deserialization errors.
    ///
    /// The prefix must not change between different versions of your
    /// application. Otherwise your application will not be able to find values
    /// created in previous versions.
    ///
    /// The prefix you set here is combined with the prefix that is configured
    /// for the entire connection in
    /// [CuttlestoreBuilder](crate::CuttlestoreBuilder). Similarly the prefix is
    /// entirely transparent to your application, you don't need to add the
    /// prefix when calling `get` or `put`, and the prefix is stripped when you
    /// `scan` the store.
    pub async fn make<Value: Serialize + DeserializeOwned + Send + Sync, C: AsRef<str>>(
        &self,
        prefix: C,
    ) -> Result<Cuttlestore<Value>, CuttlestoreError> {
        Ok(Cuttlestore {
            store: self.store.clone(),
            cleaner: self.cleaner.clone(),
            phantom: PhantomData,
            prefix: Some(format!(
                "{}:{}",
                self.prefix.clone().unwrap_or_default(),
                prefix.as_ref()
            )),
        })
    }
}

pub(crate) async fn find_matching_backend(
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
    #[cfg(feature = "backend-sqlite-core")]
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

#[cfg(test)]
mod tests {
    use crate::common::CuttlestoreError;

    use super::find_matching_backend;
    use tokio::test;

    #[test]
    async fn error_on_no_backend() {
        let result = find_matching_backend("does-not-exist://really").await;
        let error_msg = match result {
            Err(CuttlestoreError::NoMatchingBackend(msg)) => msg,
            Err(err) => panic!("Unexpected error {err:?}"),
            Ok(_) => panic!("Expected no backends, but found one"),
        };
        // The error message should not specify the part after ://, in case
        // there are passwords or other secret information there.
        assert!(
            !error_msg.contains("really"),
            "error message contains backend details"
        )
    }
}
