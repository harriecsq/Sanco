//! Sanco core: database, sync and business rules. No Tauri dependency, so
//! everything here is testable with `cargo test -p sanco-core`.

pub mod backup;
pub mod db;
pub mod domain;
pub mod error;
pub mod ids;
pub mod machine;
pub mod repo;
pub mod rng;
pub mod sync;
pub mod time;

pub use db::registry::Row;
pub use db::{Db, OpenOptions};
pub use error::{Error, Result};
pub use repo::{Actor, ChangeEvent, Origin, WriteTx};
