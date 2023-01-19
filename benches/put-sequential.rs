use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use cuttlestore::Cuttlestore;
use lipsum::lipsum;
use tokio::runtime;

/// Load the store with values to initialize it.
async fn load(store: Cuttlestore<String>, count_keys: u64) {
    for i in 0..count_keys {
        store
            .put(format!("{i}"), &lipsum(1 + (i % 30) as usize))
            .await
            .unwrap();
    }
}

fn criterion_benchmark(c: &mut Criterion) {
    let count_entries = 1000;
    let mut group = c.benchmark_group(format!("put {count_entries} entries sequentially"));
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
        store
    });
    group.bench_with_input("redis", &redis.clone(), |b, redis| {
        b.to_async(&runtime)
            .iter(|| load(redis.clone(), count_entries))
    });
    drop(redis);

    let in_memory = runtime.block_on(async {
        let store: Cuttlestore<String> = Cuttlestore::new("in-memory").await.unwrap();
        store
    });
    group.bench_with_input("in_memory", &in_memory, |b, in_memory| {
        b.to_async(&runtime)
            .iter(|| load(in_memory.clone(), count_entries))
    });
    drop(in_memory);

    std::fs::remove_dir_all("./example-store/filesystem").ok();
    let filesystem = runtime.block_on(async {
        let store: Cuttlestore<String> =
            Cuttlestore::new("filesystem://./example-store/filesystem")
                .await
                .unwrap();
        store
    });
    group.bench_with_input("filesystem", &filesystem.clone(), |b, filesystem| {
        b.to_async(&runtime)
            .iter(|| load(filesystem.clone(), count_entries))
    });
    drop(filesystem);
    std::fs::remove_dir_all("./example-store/filesystem").ok();

    std::fs::remove_file("./example-store/sqlite").ok();
    std::fs::remove_file("./example-store/sqlite-shm").ok();
    std::fs::remove_file("./example-store/sqlite-wal").ok();
    let sqlite = runtime.block_on(async {
        let store: Cuttlestore<String> = Cuttlestore::new("sqlite://example-store/sqlite")
            .await
            .unwrap();
        store
    });
    group.bench_with_input("sqlite", &sqlite.clone(), |b, sqlite| {
        b.to_async(&runtime)
            .iter(|| load(sqlite.clone(), count_entries))
    });
    drop(sqlite);
    std::fs::remove_file("./example-store/sqlite").ok();
    std::fs::remove_file("./example-store/sqlite-shm").ok();
    std::fs::remove_file("./example-store/sqlite-wal").ok();

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
