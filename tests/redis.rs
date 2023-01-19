mod tests;
use tests::suite;

use cuttlestore::Cuttlestore;
use tokio::test;

#[test]
async fn test_redis() {
    let store: Cuttlestore<String> = Cuttlestore::new("redis://127.0.0.1").await.unwrap();

    suite(&store).await;
}
