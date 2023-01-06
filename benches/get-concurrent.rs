use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use cuttlestore::Cuttlestore;
use lipsum::lipsum;
use rand::prelude::Distribution;
use tokio::{runtime, task::JoinHandle};

/// Load the store with values to initialize it.
async fn load(store: Cuttlestore<String>, count_keys: u64) {
    let mut tasks: Vec<JoinHandle<()>> = Vec::new();

    for i in 0..count_keys {
        let store = store.clone();
        tasks.push(tokio::task::spawn(async move {
            store
                .put(format!("{i}"), &lipsum(1 + (i % 30) as usize))
                .await
                .unwrap();
        }));
    }

    futures::future::try_join_all(tasks).await.unwrap();
}

async fn test(store: Cuttlestore<String>, count_keys: u64, checks: u64) {
    let mut tasks: Vec<JoinHandle<()>> = Vec::new();
    let uniform = rand::distributions::Uniform::new(0, count_keys * 2); // about 50% chance of missing
    let mut rng = rand::thread_rng();

    for _ in 0..checks {
        let store = store.clone();
        let key = uniform.sample(&mut rng);

        tasks.push(tokio::task::spawn(async move {
            let val = black_box(store.get(format!("{}", key)).await.unwrap());
            if key < count_keys {
                assert!(val.is_some());
            } else {
                assert!(val.is_none());
            }
        }));
    }

    futures::future::try_join_all(tasks).await.unwrap();
}

fn criterion_benchmark(c: &mut Criterion) {
    let count_entries = 1000;
    let count_checks = 1000;
    let mut group = c.benchmark_group(format!(
        "get {count_checks} times from keyspace of {count_entries} entries concurrently, 50% hit rate"
    ));
    group
        .measurement_time(Duration::from_secs(10))
        .sample_size(10);

    let runtime = runtime::Builder::new_multi_thread()
        .enable_time()
        .enable_io()
        .build()
        .unwrap();

    let redis = runtime.block_on(async {
        let store: Cuttlestore<String> = Cuttlestore::new("redis://127.0.0.1").await.unwrap();
        load(store.clone(), count_entries).await;
        store
    });
    group.bench_with_input("redis", &redis.clone(), |b, redis| {
        b.to_async(&runtime)
            .iter(|| test(redis.clone(), count_entries, count_checks))
    });
    drop(redis);

    let in_memory = runtime.block_on(async {
        let store: Cuttlestore<String> = Cuttlestore::new("in-memory").await.unwrap();
        load(store.clone(), count_entries).await;
        store
    });
    group.bench_with_input("in_memory", &in_memory, |b, in_memory| {
        b.to_async(&runtime)
            .iter(|| test(in_memory.clone(), count_entries, count_checks))
    });
    drop(in_memory);

    std::fs::remove_dir_all("./example-store/filesystem").ok();
    let filesystem = runtime.block_on(async {
        let store: Cuttlestore<String> =
            Cuttlestore::new("filesystem://./example-store/filesystem")
                .await
                .unwrap();
        load(store.clone(), count_entries).await;
        store
    });
    group.bench_with_input("filesystem", &filesystem.clone(), |b, filesystem| {
        b.to_async(&runtime)
            .iter(|| test(filesystem.clone(), count_entries, count_checks))
    });
    drop(filesystem);
    std::fs::remove_dir_all("./example-store/filesystem").ok();

    std::fs::remove_file("./example-store/sqlite").ok();
    std::fs::remove_file("./example-store/sqlite-journal").ok();
    let sqlite = runtime.block_on(async {
        let store: Cuttlestore<String> = Cuttlestore::new("sqlite://example-store/sqlite")
            .await
            .unwrap();
        load(store.clone(), count_entries).await;
        store
    });
    group.bench_with_input("sqlite", &sqlite.clone(), |b, sqlite| {
        b.to_async(&runtime)
            .iter(|| test(sqlite.clone(), count_entries, count_checks))
    });
    drop(sqlite);
    std::fs::remove_file("./example-store/sqlite").ok();
    std::fs::remove_file("./example-store/sqlite-journal").ok();

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
