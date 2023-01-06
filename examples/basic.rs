use cuttlestore::Cuttlestore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct User {
    name: String,
    age: u32,
}

#[tokio::main]
async fn main() {
    let store = Cuttlestore::new("filesystem://./example-store/basic")
        .await
        .unwrap();

    let value: Option<User> = store.get("foo").await.unwrap();
    println!("At first: {value:?}");

    let user = User {
        name: "Foo".to_string(),
        age: 46,
    };
    store.put("foo", &user).await.unwrap();
    let value: Option<User> = store.get("foo").await.unwrap();
    println!("After writing: {value:?}");

    store.delete("foo").await.unwrap();
    let value: Option<User> = store.get("foo").await.unwrap();
    println!("After being deleted: {value:?}");
}
