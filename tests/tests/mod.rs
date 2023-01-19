use std::time::Duration;

use cuttlestore::{Cuttlestore, PutOptions};
use futures::StreamExt;
use nanoid::nanoid;

pub async fn get_missing(store: &Cuttlestore<String>) {
    assert!(store.get(nanoid!()).await.unwrap().is_none());
}

pub async fn get_then_delete(store: &Cuttlestore<String>) {
    let key = nanoid!();
    let value = nanoid!();
    store.put(&key, &value).await.unwrap();

    let response = store.get(&key).await.unwrap();
    assert!(response.is_some());
    assert_eq!(response.unwrap(), value);
    store.delete(&key).await.unwrap();

    let response = store.get(&key).await.unwrap();
    assert!(response.is_none());
}

pub async fn timeout(store: &Cuttlestore<String>) {
    let key = nanoid!();
    store
        .put_with(&key, &nanoid!(), PutOptions::ttl_secs(3))
        .await
        .unwrap();

    let response = store.get(&key).await.unwrap();
    assert!(response.is_some());

    tokio::time::sleep(Duration::from_secs(5)).await;

    let response = store.get(&key).await.unwrap();
    assert!(response.is_none());
}

pub async fn overwrite(store: &Cuttlestore<String>) {
    let key = nanoid!();
    let value = nanoid!();
    store.put(&key, &value).await.unwrap();

    let response = store.get(&key).await.unwrap();
    assert!(response.is_some());
    assert_eq!(response.unwrap(), value);

    let new_value = nanoid!();
    store.put(&key, &new_value).await.unwrap();

    let response = store.get(&key).await.unwrap();
    assert!(response.is_some());
    assert_eq!(response.unwrap(), new_value);
}

pub async fn scan(store: &Cuttlestore<String>) {
    let (key1, value1) = (nanoid!(), nanoid!());
    let (key2, value2) = (nanoid!(), nanoid!());
    let (key3, value3) = (nanoid!(), nanoid!());

    let (r1, r2, r3) = tokio::join!(
        store.put(&key1, &value1),
        store.put(&key2, &value2),
        store.put(&key3, &value3),
    );
    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert!(r3.is_ok());

    let results = store.scan().await.unwrap();
    let results = results.map(|x| x.unwrap());
    let results = results.collect::<Vec<_>>().await;

    assert_eq!(results.len(), 3);
    assert!(results.contains(&(key1, value1)));
    assert!(results.contains(&(key2, value2)));
    assert!(results.contains(&(key3, value3)));
}

pub async fn suite(store: &Cuttlestore<String>) {
    tokio::join!(
        get_missing(&store),
        get_then_delete(&store),
        timeout(&store),
        overwrite(&store),
        scan(&store),
    );
}
