use std::time::Duration;

use cuttlestore::{Cuttlestore, PutOptions};
use nanoid::nanoid;
use tokio::task::JoinHandle;

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

pub async fn suite(store: &Cuttlestore<String>) {
    let mut tasks: Vec<JoinHandle<()>> = Vec::new();

    let s = store.clone();
    tasks.push(tokio::spawn(async move {
        get_missing(&s).await;
    }));

    let s = store.clone();
    tasks.push(tokio::spawn(async move {
        get_then_delete(&s).await;
    }));

    let s = store.clone();
    tasks.push(tokio::spawn(async move {
        timeout(&s).await;
    }));

    let s = store.clone();
    tasks.push(tokio::spawn(async move {
        overwrite(&s).await;
    }));

    futures::future::try_join_all(tasks).await.unwrap();
}
