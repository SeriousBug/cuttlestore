use cuttlestore::{Cuttlestore, CuttlestoreBuilder};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Player {
    first_name: String,
    last_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Item {
    name: String,
    value: u64,
}

#[tokio::main]
async fn main() {
    let connection = CuttlestoreBuilder::new("filesystem://./example-store/using-builder")
        // Every 3 seconds, the store will be swept for stale values.
        .clean_every_secs(3)
        // Make into a connection so we can make multiple stores
        .finish_connection()
        .await
        .unwrap();

    let player_store: Cuttlestore<Player> = connection.make("player").await.unwrap();
    let item_store: Cuttlestore<Item> = connection.make("item").await.unwrap();

    let items = vec![
        Item {
            name: "Potion".to_string(),
            value: 35,
        },
        Item {
            name: "Sword".to_string(),
            value: 340,
        },
    ];
    for item in items {
        item_store.put(&item.name, &item).await.unwrap();
    }
    player_store
        .put(
            "chieftain",
            &Player {
                first_name: "Cairne".to_string(),
                last_name: "Bloodhoof".to_string(),
            },
        )
        .await
        .unwrap();

    assert!(player_store.get("king").await.unwrap().is_none());
    assert!(player_store.get("chieftain").await.unwrap().is_some());
    // Different stores don't share keys
    assert!(item_store.get("chieftain").await.unwrap().is_none());
}
