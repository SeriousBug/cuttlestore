mod tests;
use tests::suite;

use cuttlestore::Cuttlestore;
use tokio::test;

#[test]
async fn test_dynamodb() {
    let store: Cuttlestore<String> =
        Cuttlestore::new("dynamodb://us-east-1/cuttlestore-test?endpoint=http://127.0.0.1:8000")
            .await
            .unwrap();

    suite(&store).await;
}
