use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unknown table `{0}`")]
    UnknownTable(String),
    #[error("unknown column `{table}.{column}`")]
    UnknownColumn { table: String, column: String },
    #[error("bad value for `{table}.{column}`: {reason}")]
    BadValue {
        table: String,
        column: String,
        reason: String,
    },
    #[error("`{0}` cannot be written directly")]
    ReadOnlyTable(String),
    #[error("{table} row `{id}` not found")]
    NotFound { table: String, id: String },
    #[error("{table} row `{id}` already exists")]
    AlreadyExists { table: String, id: String },
    #[error("{table} row `{id}` is deleted")]
    Deleted { table: String, id: String },

    #[error("peer clock is {ahead_ms} ms ahead of this PC")]
    ClockSkew { ahead_ms: i64 },
    #[error("schema mismatch: this PC has v{ours}, peer has v{theirs}")]
    SchemaMismatch { ours: u32, theirs: u32 },
    #[error("database schema v{found} is newer than this app (v{supported})")]
    SchemaTooNew { found: u32, supported: u32 },
    #[error("peer {site} changed epoch (restored or copied database)")]
    EpochMismatch { site: String },
    #[error("peer {site} went back in time (restored or copied database)")]
    PeerRewound { site: String },
    #[error("batch came from this PC's own site id")]
    OwnSite,
    #[error("malformed row from peer: {0}")]
    Malformed(String),

    #[error("peer {0} is unreachable")]
    PeerUnreachable(String),
    #[error("no free {series} number after {rounds} tries")]
    NumberUnavailable { series: String, rounds: usize },

    #[error("database writer stopped")]
    WriterGone,
}
