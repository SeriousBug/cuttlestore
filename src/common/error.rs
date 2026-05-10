#[derive(Debug, thiserror::Error)]
pub enum CuttlestoreError {
    /// Connection string does not specify any supported backends.
    #[error("No store matching {0} is supported.")]
    NoMatchingBackend(String),

    /// An error occurred when encoding an object.
    ///
    /// Data is encoded internally to store objects. An encoding error
    /// likely means an issue with your application's data types.
    #[error("Failed to encode data: {0}")]
    EncodingError(#[from] bincode::error::EncodeError),

    /// An error occurred when decoding an object.
    ///
    /// A decoding error could mean an issue with your application's data
    /// types, or it could point to data corruption.
    #[error("Failed to decode data: {0}")]
    DecodingError(#[from] bincode::error::DecodeError),

    /// An error happened when accessing the file system.
    ///
    /// The filesystem backend needs to be able to read and write files inside
    /// the configured path. It also attempts to create this folder and any
    /// parent folder if missing. Make sure the program has write access to the
    /// folder and disk space is available.
    #[cfg(feature = "backend-filesystem")]
    #[error("Failed to access the file system: {0}")]
    FileError(#[from] std::io::Error),

    /// An error happened when opening or accessing the sqlite database.
    #[cfg(feature = "backend-sqlite-core")]
    #[error("Failed to access the sqlite database: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[cfg(feature = "backend-redis")]
    #[error("Failed to access redis: {0}")]
    RedisError(#[from] redis::RedisError),

    #[cfg(feature = "backend-redis")]
    #[error("Failed to connecting to or accessing redis: {0}")]
    RedisConnectionError(#[from] bb8::RunError<redis::RedisError>),

    /// An error happened when accessing DynamoDB.
    #[cfg(feature = "backend-dynamodb")]
    #[error("Failed to access DynamoDB: {0}")]
    DynamoDBError(String),

    /// An error happened when communicating with the CouchDB server over HTTP.
    #[cfg(feature = "backend-couchdb-core")]
    #[error("Failed to access CouchDB: {0}")]
    CouchdbHttpError(#[from] reqwest::Error),

    /// CouchDB returned an unexpected status code or error response.
    #[cfg(feature = "backend-couchdb-core")]
    #[error("CouchDB error: {0}")]
    CouchdbError(String),

    /// Failed to parse JSON received from a CouchDB server.
    #[cfg(feature = "backend-couchdb-core")]
    #[error("Failed to parse JSON: {0}")]
    JsonError(#[from] serde_json::Error),
}
