//! Cuttlestore is a generic API for key-value stores that can be backed by
//! different storage backends. Cuttlestore allows you to build applications
//! that can be deployed to use different storage options: a small deployment
//! can use an Sqlite backend to simplify operations while a large deployment
//! can use Redis for better scalability.
//!
//! ```
//! use cuttlestore::{Cuttlestore, PutOptions};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! struct SelfDestructingMessage {
//!     message: String,
//! }
//!
//! # tokio_test::block_on(async {
//! let store: Cuttlestore<SelfDestructingMessage> = Cuttlestore::new("redis://127.0.0.1")
//!     .await
//!     .unwrap();
//!
//! let mission = SelfDestructingMessage {
//!   message: "Your mission, should you choose to accept it, ...".to_string(),
//! };
//! store
//!   .put_with("impossible", &mission, PutOptions::ttl_secs(60))
//!   .await
//!   .unwrap();
//!
//! // Later
//!
//! let value: Option<SelfDestructingMessage> = store.get("impossible").await.unwrap();
//! println!("Message says: {value:?}");
//! # })
//! ```
mod api;
mod backend_api;
mod backends;
mod builder;
mod common;

pub use api::Cuttlestore;
pub use backend_api::PutOptions;
pub use builder::CuttlestoreBuilder;
