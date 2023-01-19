mod tests;
use tests::suite;

use cuttlestore::Cuttlestore;
use tokio::{fs, test};

#[test]
async fn test_sqlite() {
    let store: Cuttlestore<String> = Cuttlestore::new("in-memory").await.unwrap();

    suite(&store).await;
}
