use std::{collections::HashMap, pin::Pin};

use async_trait::async_trait;
use bb8::Pool;
use bb8_redis::RedisConnectionManager;

use futures::{stream::BoxStream, Stream};
use lazy_regex::regex_captures;
use redis::{AsyncCommands, IntoConnectionInfo, RedisError};
use tokio::{sync::mpsc::Receiver, task::JoinHandle};

use crate::{
    backend_api::{CuttleBackend, PutOptions},
    common::CuttlestoreError,
};

pub(crate) struct RedisBackend {
    pool: Pool<RedisConnectionManager>,
}

impl RedisBackend {
    async fn new(address: &str, args: HashMap<&str, &str>) -> Result<Box<Self>, CuttlestoreError> {
        let mut info = address.into_connection_info()?;

        info.redis.username = args.get("username").map(|v| v.to_string());
        info.redis.password = args.get("password").map(|v| v.to_string());

        let manager = RedisConnectionManager::new(info)?;
        let pool = Pool::builder().build(manager).await?;

        Ok(Box::new(RedisBackend { pool }))
    }
}

#[async_trait]
impl CuttleBackend for RedisBackend {
    async fn new(conn: &str) -> Option<Result<Box<Self>, CuttlestoreError>> {
        if let Some((_, secure, address, args)) =
            regex_captures!(r#"^redis(s)?://([^?]+)[?]?(.*)"#, conn)
        {
            let arg_pairs: HashMap<&str, &str> = args
                .split("&")
                .map(|pair| pair.split_once("="))
                .filter_map(|v| v)
                .collect();

            let address = format!("redis{secure}://{address}");

            Some(RedisBackend::new(&address, arg_pairs).await)
        } else {
            None
        }
    }

    fn requires_cleaner(&self) -> bool {
        false
    }

    fn name(&self) -> &'static str {
        "redis"
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CuttlestoreError> {
        let mut connection = self.pool.get().await?;
        let payload: Option<Vec<u8>> = connection.get(key).await?;
        Ok(payload)
    }

    async fn put(
        &self,
        key: &str,
        value: &[u8],
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        let mut connection = self.pool.get().await?;

        if let Some(ttl) = options.ttl {
            let _: () = connection.set_ex(key, value, ttl as usize).await?;
        } else {
            let _: () = connection.set(key, value).await?;
        }

        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), CuttlestoreError> {
        let mut connection = self.pool.get().await?;

        let _: () = connection.del(key).await?;

        Ok(())
    }

    async fn scan(
        &self,
    ) -> Result<BoxStream<Result<(String, Vec<u8>), CuttlestoreError>>, CuttlestoreError> {
        Ok(Box::pin(RedisScanStream::new(self.pool.clone()).await))
    }
}

// The redis client needs/wants to keep the same connection open throughout the
// scan, but I couldn't get that working with the stream because the stream
// doesn't own the connection. I think that was the problem.
//
// This gets around the problem by creating a Tokio task which performs the scan
// and queues up the values into a channel, which can then be consumed to
// produce the stream. The connection is owned by the same stream throughout so
// it works fine.
//
// I'm not fully sure it has to be this way, maybe I'm too sleep deprived and
// couldn't figure it out. Contributions welcome!
struct RedisScanStream {
    handle: JoinHandle<()>,
    rx: Receiver<Result<(String, Vec<u8>), CuttlestoreError>>,
}

impl RedisScanStream {
    async fn new(pool: Pool<RedisConnectionManager>) -> Self {
        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<(String, Vec<u8>), CuttlestoreError>>(10);
        let handle = tokio::spawn(async move {
            // Handle the setup, send back an error and exit early if it fails.
            let mut connection = match pool.get().await {
                Ok(connection) => connection,
                Err(err) => {
                    tx.send(Err(err.into())).await.ok();
                    return;
                }
            };
            let mut keys = match connection.scan::<String>().await {
                Ok(keys) => keys,
                Err(err) => {
                    tx.send(Err(err.into())).await.ok();
                    return;
                }
            };

            // Start putting the pairs through
            loop {
                let mut connection = match pool.get().await {
                    Ok(connection) => connection,
                    Err(err) => {
                        tx.send(Err(err.into())).await.ok();
                        return;
                    }
                };
                if let Some(key) = keys.next_item().await {
                    let value: Result<Option<Vec<u8>>, RedisError> = connection.get(&key).await;
                    match value {
                        Ok(Some(value)) => {
                            tx.send(Ok((key, value))).await.ok();
                        }
                        Ok(None) => {
                            continue;
                        }
                        Err(err) => {
                            tx.send(Err(err.into())).await.ok();
                        }
                    }
                } else {
                    return;
                }
            }
        });
        Self { handle, rx }
    }
}

impl Drop for RedisScanStream {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl<'a> Stream for RedisScanStream {
    type Item = Result<(String, Vec<u8>), CuttlestoreError>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let mut s = self;
        s.rx.poll_recv(cx)
    }
}
