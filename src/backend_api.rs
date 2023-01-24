use std::{
    borrow::Cow,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::common::{get_system_time, CuttlestoreError};

/// Options to use when placing a value into a store.
///
/// The only option is a TTL value for now, but more options may be added in the
/// future.
#[derive(Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct PutOptions {
    /// The number of seconds from the time of the store for which the data should be available.
    ///
    /// For example, setting this to 60 means the data will be available for the
    /// next minute.
    pub(crate) ttl: Option<u64>,
}

impl PutOptions {
    /// The value will be alive for this much time.
    pub fn ttl(duration: Duration) -> Self {
        Self::ttl_secs(duration.as_secs())
    }

    /// The value will be alive for this many seconds.
    pub fn ttl_secs(seconds: u64) -> Self {
        PutOptions { ttl: Some(seconds) }
    }

    /// The value will be alive until this time.
    ///
    /// This function will panic if a time before the `UNIX_EPOCH` is used.
    /// Unless you are a time traveler it doesn't make sense use dates in the
    /// past anyway, so avoid doing that!
    ///
    /// In some cases the actual TTL may end up being a second after this time.
    /// If the exact second is important, you may want to add a more accurate
    /// timestamp into your value and check that yourself.
    pub fn live_until(time: SystemTime) -> Self {
        PutOptions {
            ttl: Some(
                time.duration_since(UNIX_EPOCH)
                    .expect("The live_until time for a value is before the UNIX epoch")
                    .as_secs()
                    - get_system_time(),
            ),
        }
    }
}

#[allow(clippy::derivable_impls)]
// While the Default derivation for `Option` is `None`, we want to explicitly
// state that here.
impl Default for PutOptions {
    /// By default, there is no TTL. Values will live indefinitely.
    fn default() -> Self {
        Self { ttl: None }
    }
}

/// The common API for Cuttlestore backends.
///
/// This API defines the contract between Cuttlestore and the backends. Backends
/// must implement this API, and follow the requirements when doing so.
#[async_trait]
pub(crate) trait CuttleBackend {
    /// If the given connection string matches this backend, create the backend.
    ///
    /// The backend MUST return a `None` if the connection string does not
    /// match, even if errors occurred when trying to check for a match.
    async fn new(conn: &str) -> Option<Result<Box<Self>, CuttlestoreError>>
    where
        // Do not allow `new` to be called from trait objects, because you can't
        Self: Sized;

    /// If the backend does not have built-in support for cleaning up expired
    /// pairs, then the backend MUST return true to enable an external cleaner.
    fn requires_cleaner(&self) -> bool;

    /// The name for the backend.
    fn name(&self) -> &'static str;

    /// Get a value out of the store.
    ///
    /// The backend MUST NOT return the value if the ttl was provided at the
    /// time of the store, and the ttl has expired.
    async fn get<'a>(&self, key: Cow<'a, str>) -> Result<Option<Vec<u8>>, CuttlestoreError>;
    /// Put a value into the store.
    async fn put<'a>(
        &self,
        key: Cow<'a, str>,
        value: &[u8],
        options: PutOptions,
    ) -> Result<(), CuttlestoreError>;
    /// Delete a value from the store.
    async fn delete<'a>(&self, key: Cow<'a, str>) -> Result<(), CuttlestoreError>;
    /// Walk through all the key-value pairs in the store.
    ///
    /// The backend MUST NOT return the pair if the ttl was provided at the time
    /// of the store, and the ttl has expired.
    ///
    /// If the backend requires an external cleaner (i.e. requires_cleaner
    /// returns true), then this function MUST delete expired pairs when they
    /// are encountered.
    async fn scan(
        &self,
    ) -> Result<BoxStream<Result<(String, Vec<u8>), CuttlestoreError>>, CuttlestoreError>;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::PutOptions;

    #[test]
    fn seconds_are_seconds() {
        assert_eq!(
            PutOptions::ttl_secs(32),
            PutOptions::ttl(Duration::from_secs(32))
        )
    }
}
