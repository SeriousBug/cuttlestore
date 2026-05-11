use std::{borrow::Cow, collections::HashMap, pin::Pin};

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
        let info = address.into_connection_info()?;

        let mut redis_settings = info.redis_settings().clone();
        if let Some(username) = args.get("username") {
            redis_settings = redis_settings.set_username(username);
        }
        if let Some(password) = args.get("password") {
            redis_settings = redis_settings.set_password(password);
        }
        let info = info.set_redis_settings(redis_settings);

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
                .split('&')
                .flat_map(|pair| pair.split_once('='))
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

    async fn get<'a>(
        &'a self,
        key: Cow<'a, str>,
    ) -> Result<Option<Cow<'a, [u8]>>, CuttlestoreError> {
        let mut connection = self.pool.get().await?;
        let payload: Option<Vec<u8>> = connection.get(key.as_ref()).await?;
        Ok(payload.map(Cow::Owned))
    }

    async fn put<'a>(
        &self,
        key: Cow<'a, str>,
        value: &[u8],
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        let mut connection = self.pool.get().await?;

        if let Some(ttl) = options.ttl {
            let _: () = connection.set_ex(key.as_ref(), value, ttl).await?;
        } else {
            let _: () = connection.set(key.as_ref(), value).await?;
        }

        Ok(())
    }

    async fn delete<'a>(&self, key: Cow<'a, str>) -> Result<(), CuttlestoreError> {
        let mut connection = self.pool.get().await?;

        let _: () = connection.del(key.as_ref()).await?;

        Ok(())
    }

    async fn scan<'a>(
        &'a self,
    ) -> Result<
        BoxStream<'a, Result<(String, Cow<'a, [u8]>), CuttlestoreError>>,
        CuttlestoreError,
    > {
        use futures::StreamExt;
        let stream = RedisScanStream::new(self.pool.clone())
            .await
            .map(|item| item.map(|(k, v)| (k, Cow::Owned(v))));
        Ok(Box::pin(stream))
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
                    let key = match key {
                        Ok(key) => key,
                        Err(err) => {
                            tx.send(Err(err.into())).await.ok();
                            return;
                        }
                    };
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

impl Stream for RedisScanStream {
    type Item = Result<(String, Vec<u8>), CuttlestoreError>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let mut s = self;
        s.rx.poll_recv(cx)
    }
}
