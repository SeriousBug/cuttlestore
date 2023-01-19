mod tests;
use tests::suite;

use cuttlestore::{Cuttlestore, CuttlestoreBuilder};
use tokio::test;

#[test]
async fn test_in_memory_with_cleaner() {
    let store: Cuttlestore<String> = CuttlestoreBuilder::new("in-memory")
        .clean_every_secs(1)
        .finish()
        .await
        .unwrap();

    suite(&store).await;
}
