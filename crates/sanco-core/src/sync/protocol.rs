//! Wire types exchanged between peers.

use serde::{Deserialize, Serialize};

use crate::db::registry::Row;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireRow {
    /// Table name.
    pub t: String,
    /// Complete row, meta columns included.
    pub row: Row,
}

/// Answer to a pull: rows changed on the sender after `since`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeBatch {
    pub site: String,
    pub epoch: String,
    pub schema: u32,
    /// The sender's latest version number when the batch was read.
    pub head: i64,
    /// Watermark to store once this batch is applied.
    pub upto: i64,
    /// More rows remain after `upto`.
    pub more: bool,
    pub rows: Vec<WireRow>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyReport {
    pub accepted: usize,
    pub ignored: usize,
    pub conflicts: usize,
}
