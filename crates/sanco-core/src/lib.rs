//! Sanco core: database, sync and business rules. No Tauri dependency, so
//! everything here is testable with `cargo test -p sanco-core`.

pub mod db;
pub mod error;
pub mod ids;
pub mod repo;
pub mod rng;
pub mod sync;
pub mod time;

pub use db::registry::Row;
pub use db::{Db, OpenOptions};
pub use error::{Error, Result};
pub use repo::{Actor, ChangeEvent, Origin, WriteTx};

/// Greeting used by the M0 end-to-end check.
pub fn greeting(name: &str) -> String {
    let name = name.trim();
    let name = if name.is_empty() { "there" } else { name };
    format!("Hello, {name}! Sanco core is connected.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_trims_and_defaults() {
        assert_eq!(greeting("  Ana "), "Hello, Ana! Sanco core is connected.");
        assert_eq!(greeting(""), "Hello, there! Sanco core is connected.");
    }
}
