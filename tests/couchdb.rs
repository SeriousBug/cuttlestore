mod tests;
use tests::suite;

use cuttlestore::Cuttlestore;
use tokio::test;

#[test]
async fn test_couchdb() {
    // CouchDB doesn't allow database names to start with a number, so use a
    // unique alphabetic name per test run to keep tests isolated.
    let db = format!(
        "cuttlestore_test_{}",
        nanoid::nanoid!(
            16,
            &[
                'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p',
                'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z'
            ]
        )
    );
    let conn = format!("couchdb://admin:password@127.0.0.1:5984/{db}");
    let store: Cuttlestore<String> = Cuttlestore::new(conn).await.unwrap();

    suite(&store).await;
}
