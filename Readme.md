# [Cuttlestore](https://github.com/SeriousBug/Cuttlestore)

[![tests](https://img.shields.io/github/actions/workflow/status/SeriousBug/cuttlestore/test.yml?label=tests&branch=main)](https://github.com/SeriousBug/cuttlestore/actions/workflows/test.yml)
[![Test coverage report](https://img.shields.io/codecov/c/github/SeriousBug/cuttlestore)](https://codecov.io/gh/SeriousBug/cuttlestore)
[![lint checks](https://img.shields.io/github/actions/workflow/status/SeriousBug/cuttlestore/lint.yml?label=lints&branch=main)](https://github.com/SeriousBug/cuttlestore/actions/workflows/lint.yml)
[![Releases](https://img.shields.io/github/v/release/SeriousBug/cuttlestore?include_prereleases)](https://github.com/SeriousBug/cuttlestore/releases)
[![MIT license](https://img.shields.io/github/license/SeriousBug/cuttlestore)](https://github.com/SeriousBug/cuttlestore/blob/main/LICENSE.txt)

Cuttlestore is a generic API for key-value stores. It allows you to support
multiple key-value stores with zero additional effort, and makes it possible to
switch between different stores at runtime.

## Example

```rust
use cuttlestore::{Cuttlestore, PutOptions};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
struct SelfDestructingMessage {
    message: String,
}

#[tokio::main]
async fn main() {
    let store = Cuttlestore::new("filesystem://./example-store")
                                // or redis, sqlite, in-memory
        .await
        .unwrap();

    let mission = SelfDestructingMessage {
        message: "Your mission, should you choose to accept it, ...".to_string(),
    };
    store
        .put_with("impossible", &mission, PutOptions::ttl_secs(60))
        .await
        .unwrap();

    // Later

    let value: Option<SelfDestructingMessage> = store.get("impossible").await.unwrap();
    println!("Message says: {value:?}");
}
```

## Supported Backends

Cuttlestore currently has support for:

| Name       | Feature            | Connection string | Description                                                                                     |
| ---------- | ------------------ | ----------------- | ----------------------------------------------------------------------------------------------- |
| Redis      | backend-redis      | redis://127.0.0.1 | Backed by Redis. This will get you the best scalability.                                        |
| Sqlite     | backend-sqlite     | sqlite://path     | An sqlite database used as a key-value store. Best performance if scalability is not a concern. |
| Filesystem | backend-filesystem | filesystem://path | Uses files in a folder as a key-value store. Performance depends on your filesystem.            |
| In-Memory  | backend-in-memory  | in-memory         | Not persistent, but very high performance. Useful if the store is ephemeral, like a cache.      |

## Installing

Add Cuttlestore to your `Cargo.toml`:

```toml
cuttlestore = "0.1"
```

## Overview

- **Pros:** Cuttlestore is useful if
  - You want to allow end-users to pick which store to use without recompiling
  - You are looking for a simple API for a basic key-value store
- **Cons:** Avoid Cuttlestore if:
  - You need access to key-value store specific features
  - You only want to one key-value store, and don't care about switching

For example, if you are making a self-hostable web application, and you want to
allow users to pick between using Redis and sqlite depending on their needs, you
could use Cuttlestore. Cuttlestore supports both of these backends, and your
users could input the connection string in your application settings to pick one
of these backends. Users with large deployments could pick Redis, and
small-scale users could pick sqlite so they don't have to deal with also
deploying Redis.

## Logging

The library can log errors with both
[tracing](https://docs.rs/tracing/latest/tracing/) and
[log](https://docs.rs/log/latest/log/). `tracing` is enabled by default, but you
can switch to `log` by enabling the feature:

```toml
cuttlestore = { version = "0.1", default-features = false, features = [
    "logging-log",
    # remember to enable the backends!
    "backend-redis",
    "backend-sqlite",
    "backend-filesystem",
    "backend-in-memory",
] }
```

## Details of backends

### Redis

Redis is generally the best option if you don't mind setting it up. It offers
good performance and scalability as you can connect many app servers into the
same Redis instance.

Cuttlestore has support for TLS, which you can activate by adding an `s` to the
connection string like `rediss://127.0.0.1`. You can also change the port you are
using by adding `:port` to the end, for example `redis://127.0.0.1:5678`.

Cuttlestore has support for ACLs as well. You can enable them by adding them to
the connection string. For example, if your username is `agent` and password is
`47`, you can use the connection string
`redis://127.0.0.1?username=agent&password=47`.

### Sqlite

Cuttlestore can use an sqlite database as a key-value store when using this
backend. The database and any tables are automatically created.

The sqlite database is configured to use write ahead logging, which means it may
create some additional files next to the database file you configure in the
connection string. The configuration is also set in a way that there is a small
chance of losing the last few `put` or `delete` operations if a crash occurs,
which is unfortunately required to bring the performance to a reasonable level.

Sqlite doesn't have built-in ttl support, so ttl is supported by periodically
scanning the database and deleting expired entries on a best-effort basis. This
scan uses a Tokio task, meaning it will run within your existing Tokio thread
pool.

### Filesystem

Cuttlestore can be configured to use a folder as a key value store. When using
this backend, the file names in the folder are the keys, and the values are
stored using a binary encoding within the files.

The performance largely depends on your filesystem. Durability is similar to
sqlite: there is a small risk of losing the latest few operations, but data
corruption is not expected.

The ttl feature is supported by periodically scanning the database and deleting
expired entries on a best-effort basis. This scan uses a Tokio task, meaning it
will run within your existing Tokio thread pool.

#### In-Memory

The in-memory backend is a multithreaded in-memory key-value store backed by
[dashmap](https://docs.rs/dashmap/latest/dashmap/index.html).

The performance is the best, but everything is kept in-memory so there is no
durability.

## TTL

The TTL (time to live) feature allows you to designate values that should only
exist in the store for a limited amount of time. The values that run out of TTL
will be expired and deleted from the store to save space.

```rust
store.put_with("impossible", &mission, PutOptions::ttl_secs(60))
// or
store.put_with("impossible", &mission, PutOptions::ttl(Duration::from_secs(60)))
```

Some backends have built-in support for TTLs (redis). For other backends, the
TTL support is emulated by periodically running a Tokio task which scans the
store and cleans up expired values. This task runs within your existing Tokio
thread pool. You can configure how often this cleanup task runs using
`CuttlestoreBuilder`, see the [builder example](https://github.com/SeriousBug/cuttlestore/blob/main/examples/using-builder.rs#L13-L18).

Get and scan operations are guaranteed to never return expired values, but
expired values are not necessarily deleted immediately.

## Benchmarks

There are some [benchmarks to compare the performance of the different backends](https://seriousbug.github.io/cuttlestore/reports/).
All the existing benchmarks use small keyspaces so the performance is not
necessarily realistic.

The concurrent benchmarks show you the overall throughput of the backend, while
the sequential benchmarks show you the average latency you can expect from each
request.

In short, these benchmarks show a few things:

- Redis has relatively high latency, around 67 microseconds per operation. In
  comparison, the second slowest is sqlite with around 13 to 23 microseconds per
  operation.
- The filesystem backend offers the best performance both for throughput and
  latency, but there is a very large spread between the lows and highs. At worst
  case, it is slower than all other backends.
- In-memory offers incredible performance, but obviously not durable.

These benchmarks validate the suggestions listed earlier in the readme. Redis is
a good option if you need scalability, and sqlite is good if scalability is not
a concern. Filesystem can be an option if performance is not critical, but there
is risk that it will not perform well for large key spaces.
