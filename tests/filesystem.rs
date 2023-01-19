mod tests;
use tests::suite;

use cuttlestore::Cuttlestore;
use tokio::{fs, test};

#[test]
async fn test_filesystem() {
    fs::remove_dir_all("./example-store/filesystem-test")
        .await
        .ok();

    let store: Cuttlestore<String> =
        Cuttlestore::new("filesystem://./example-store/filesystem-test")
            .await
            .unwrap();

    suite(&store).await;

    fs::remove_dir_all("./example-store/filesystem-test")
        .await
        .ok();
}
