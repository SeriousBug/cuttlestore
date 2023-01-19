use cuttlestore::{Cuttlestore, CuttlestoreBuilder, PutOptions};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
struct SelfDestructingMessage {
    message: String,
}

#[tokio::main]
async fn main() {
    let store: Cuttlestore<SelfDestructingMessage> =
        CuttlestoreBuilder::new("filesystem://./example-store/using-builder")
            // Every 3 seconds, the store will be sweeped for stale values.
            .clean_every_secs(3)
            .finish()
            .await
            .unwrap();

    let value: Option<SelfDestructingMessage> = store.get("impossible").await.unwrap();
    println!("At first: {value:?}");

    let mission = SelfDestructingMessage {
        message: "Your mission, should you choose to accept it, ...".to_string(),
    };
    store
        .put_with("impossible", &mission, PutOptions::ttl_secs(1))
        .await
        .unwrap();
    let value: Option<SelfDestructingMessage> = store.get("impossible").await.unwrap();
    println!("After writing: {value:?}");

    tokio::time::sleep(Duration::from_secs(4)).await;

    let value: Option<SelfDestructingMessage> = store.get("impossible").await.unwrap();
    println!("After waiting: {value:?}");
}
