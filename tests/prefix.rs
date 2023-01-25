use futures::StreamExt;

use cuttlestore::{Cuttlestore, CuttlestoreBuilder};
use tokio::test;

#[test]
async fn test_in_memory() {
    let connection = CuttlestoreBuilder::new("in-memory")
        .prefix("foo")
        .finish_connection()
        .await
        .unwrap();

    let store_str: Cuttlestore<String> = connection.make("str").await.unwrap();
    let store_int: Cuttlestore<i64> = connection.make("int").await.unwrap();

    store_str.put("bar", &"baz".to_string()).await.unwrap();
    store_int.put("bar", &32).await.unwrap();

    assert_eq!(store_str.get("bar").await.unwrap().unwrap(), "baz");
    assert_eq!(store_int.get("bar").await.unwrap().unwrap(), 32);

    let pairs = store_str.scan().await.unwrap();
    let pairs = pairs.map(|x| x.unwrap());
    let pairs = pairs.collect::<Vec<_>>().await;

    assert_eq!(pairs.len(), 1);
    assert_eq!(
        pairs.get(0).unwrap(),
        &("bar".to_string(), "baz".to_string())
    );
}
