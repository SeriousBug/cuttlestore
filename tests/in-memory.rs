mod tests;
use tests::suite;

use cuttlestore::Cuttlestore;
use tokio::test;

#[test]
async fn test_in_memory() {
    let store: Cuttlestore<String> = Cuttlestore::new("in-memory").await.unwrap();

    suite(&store).await;
}
