mod tests;
use tests::suite;

use cuttlestore::Cuttlestore;
use tokio::{fs, test};

#[test]
async fn test_sqlite() {
    fs::remove_file("./example-store/sqlite-test").await.ok();
    fs::remove_file("./example-store/sqlite-test-shm")
        .await
        .ok();
    fs::remove_file("./example-store/sqlite-test-wal")
        .await
        .ok();

    let store: Cuttlestore<String> = Cuttlestore::new("sqlite://./example-store/sqlite-test")
        .await
        .unwrap();

    suite(&store).await;

    fs::remove_file("./example-store/sqlite-test").await.ok();
    fs::remove_file("./example-store/sqlite-test-shm")
        .await
        .ok();
    fs::remove_file("./example-store/sqlite-test-wal")
        .await
        .ok();
}
