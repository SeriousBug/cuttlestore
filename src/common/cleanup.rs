use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use tokio::task::JoinHandle;

use crate::backend_api::CuttleBackend;

pub(crate) struct CleanerOptions {
    pub(crate) sweep_every: Duration,
}

pub(crate) struct Cleaner {
    handle: JoinHandle<()>,
}

impl Cleaner {
    /// Create and start the cleaner.
    ///
    /// The cleaner will periodically sweep the store for expired keys.
    pub(crate) fn new(
        store: Arc<Box<dyn CuttleBackend + Send + Sync>>,
        options: CleanerOptions,
    ) -> Self {
        let j = tokio::spawn(async move {
            tokio::time::sleep(options.sweep_every).await;
            // should just log this and retry later.
            let scan = match store.scan().await {
                Ok(scan) => scan,
                Err(err) => {
                    #[cfg(feature = "logging-log")]
                    log::error!("Unable to run the store cleaner: {err:?}");
                    #[cfg(feature = "logging-tracing")]
                    tracing::error!("Unable to run the store cleaner: {err:?}");
                    return;
                }
            };
            // We don't need to actually do anything to check the TTLs.
            // The scan will automatically discard expired values.
            scan.for_each(|_| futures::future::ready(())).await;
        });
        Cleaner { handle: j }
    }
}

impl Drop for Cleaner {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
