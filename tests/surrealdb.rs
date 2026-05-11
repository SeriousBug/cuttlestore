mod tests;
use tests::suite;

use cuttlestore::Cuttlestore;
use tokio::test;

#[test]
async fn test_surrealdb() {
    // Use a unique database per test run so concurrent tests do not collide
    // when sharing a single SurrealDB instance.
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
    let conn = format!("surrealdb://root:root@127.0.0.1:8001/cuttlestore/{db}");
    let store: Cuttlestore<String> = Cuttlestore::new(conn).await.unwrap();

    suite(&store).await;
}
