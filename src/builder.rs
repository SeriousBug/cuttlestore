use std::time::Duration;

use serde::{de::DeserializeOwned, Serialize};

use crate::{
    common::{cleanup::CleanerOptions, CuttlestoreError},
    Cuttlestore,
};

#[derive(Debug)]
/// Configure a Cuttlestore.
pub struct CuttlestoreBuilder {
    conn: String,
    cleaner: CleanerOptions,
}

impl CuttlestoreBuilder {
    /// Start configuring your Cuttlestore.
    pub fn new<C: AsRef<str>>(conn: C) -> Self {
        Self {
            conn: conn.as_ref().to_string(),
            cleaner: CleanerOptions::default(),
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

    /// Finish configuring your Cuttlestore, finalizing it so you can use it.
    pub async fn finish<Value: Serialize + DeserializeOwned + Send + Sync>(
        self,
    ) -> Result<Cuttlestore<Value>, CuttlestoreError> {
        Cuttlestore::make(&self.conn, self.cleaner).await
    }
}
