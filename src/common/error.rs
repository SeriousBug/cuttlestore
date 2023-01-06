use std::io;

#[derive(Debug, thiserror::Error)]
pub enum CuttlestoreError {
    /// Connection string does not specify any supported backends.
    #[error("No store matching {0} is supported.")]
    NoMatchingBackend(String),

    /// An error occurred when encoding or decoding an object.
    ///
    /// Data is encoded and decoded internally to store objects. An encoding
    /// error could mean an issue with your application's data types, or it
    /// could point to data corruption.
    #[error("Failed to encode or decode data: {0}")]
    EncodingError(#[from] bincode::Error),

    /// An error happened when accessing the file system.
    ///
    /// The filesystem backend needs to be able to read and write files inside
    /// the configured path. It also attempts to create this folder and any
    /// parent folder if missing. Make sure the program has write access to the
    /// folder and disk space is available.
    #[cfg(feature = "backend-filesystem")]
    #[error("Failed to access the file system: {0}")]
    FileError(#[from] io::Error),

    /// An error happened when opening or accessing the sqlite database.
    #[cfg(feature = "backend-sqlite")]
    #[error("Failed to access the sqlite database: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[cfg(feature = "backend-redis")]
    #[error("Failed to access redis: {0}")]
    RedisError(#[from] redis::RedisError),

    #[cfg(feature = "backend-redis")]
    #[error("Failed to connecting to or accessing redis: {0}")]
    RedisConnectionError(#[from] bb8::RunError<redis::RedisError>),
}
