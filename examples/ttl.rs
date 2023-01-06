use cuttlestore::{Cuttlestore, PutOptions};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
struct SelfDestructingMessage {
    message: String,
}

#[tokio::main]
async fn main() {
    let store = Cuttlestore::new("filesystem://./example-store/ttl")
        .await
        .unwrap();

    let value: Option<SelfDestructingMessage> = store.get("impossible").await.unwrap();
    println!("At first: {value:?}");

    let mission = SelfDestructingMessage {
        message: "Your mission, should you choose to accept it, ...".to_string(),
    };
    store
        .put_with("impossible", &mission, PutOptions::ttl_secs(60))
        .await
        .unwrap();
    let value: Option<SelfDestructingMessage> = store.get("impossible").await.unwrap();
    println!("After writing: {value:?}");

    tokio::time::sleep(Duration::from_secs(4)).await;

    let value: Option<SelfDestructingMessage> = store.get("impossible").await.unwrap();
    println!("After waiting: {value:?}");
}
