use async_stream::try_stream;
use async_trait::async_trait;
use futures::stream::BoxStream;
use lazy_regex::regex_captures;
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, io::ErrorKind, path::PathBuf, str::FromStr};
use tokio::io::AsyncWriteExt;
use tokio_stream::wrappers::ReadDirStream;

use crate::{
    backend_api::{CuttleBackend, PutOptions},
    common::{get_system_time, CuttlestoreError},
};

#[derive(Debug, Serialize, Deserialize)]
struct StoredValue<'t> {
    payload: &'t [u8],
    live_until: Option<u64>,
}

pub(crate) struct FilesystemBackend {
    base_folder: PathBuf,
}

impl FilesystemBackend {
    async fn new(base_folder: &str) -> Result<Box<FilesystemBackend>, CuttlestoreError> {
        tokio::fs::create_dir_all(base_folder).await?;
        Ok(Box::new(FilesystemBackend {
            base_folder: PathBuf::from_str(base_folder).expect("Unable to get the file path"),
        }))
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CuttlestoreError> {
        match tokio::fs::read(self.base_folder.join(key)).await {
            Ok(read) => {
                let value: StoredValue = match bincode::deserialize(&read) {
                    Ok(val) => val,
                    Err(err) => {
                        // Decoding failed. This likely suggests data corruption,
                        // such as power loss while file was being written out.
                        #[cfg(feature = "logging-log")]
                        log::error!("Found potential data corruption for key {key}: {err:?}");
                        #[cfg(feature = "logging-tracing")]
                        tracing::error!("Found potential data corruption for key {key}: {err:?}");
                        self.delete(key).await?;
                        return Ok(None);
                    }
                };

                // If ttl is enabled, check in case it is expired
                if let Some(live_until) = value.live_until {
                    if live_until < get_system_time() {
                        // If the value we got is expired, discard it
                        self.delete(key).await?;
                        return Ok(None);
                    }
                }

                Ok(Some(value.payload.to_owned()))
            }
            Err(error) => match error.kind() {
                // A not found IO error just means the key is missing, it's not a real error
                ErrorKind::NotFound => Ok(None),
                _ => Err(error)?,
            },
        }
    }

    async fn delete(&self, key: &str) -> Result<(), CuttlestoreError> {
        tokio::fs::remove_file(self.base_folder.join(key)).await?;
        Ok(())
    }
}

#[async_trait]
impl CuttleBackend for FilesystemBackend {
    async fn new(conn: &str) -> Option<Result<Box<Self>, CuttlestoreError>> {
        if let Some((_, base_folder)) = regex_captures!(r#"^filesystem://(.+)"#, conn) {
            Some(FilesystemBackend::new(base_folder).await)
        } else {
            None
        }
    }

    fn requires_cleaner(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "filesystem"
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CuttlestoreError> {
        Ok(self.get(key).await?)
    }

    async fn put(
        &self,
        key: &str,
        value: &[u8],
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        let encoded_value = bincode::serialize(&StoredValue {
            payload: value,
            live_until: options.ttl.map(|v| v + get_system_time()),
        })?;

        let mut file = tokio::fs::File::create(self.base_folder.join(key)).await?;
        file.write_all(&encoded_value[..]).await?;
        //file.sync_data().await?;

        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), CuttlestoreError> {
        self.delete(key).await?;
        Ok(())
    }

    async fn scan(
        &self,
    ) -> Result<BoxStream<Result<(String, Vec<u8>), CuttlestoreError>>, CuttlestoreError> {
        let read = tokio::fs::read_dir(&self.base_folder).await?;
        let dir_entries = ReadDirStream::new(read);

        Ok(Box::pin(try_stream! {
          for await entry in dir_entries {
            let entry = entry?;
            let key = os_string_to_string(&entry.file_name());

            let value = self.get(&key).await?;
            if let Some(value) = value {
                yield (key, value);
            }
          }
        }))
    }
}

/// Makes a lossy conversion from an OS string to a regular string.
///
/// The lossiness is not a problem for us, because we wouldn't have created a
/// file that contains a character that can't be represented in a regular string
/// anyway.
fn os_string_to_string(os_str: &OsString) -> String {
    os_str.to_string_lossy().to_string()
}
