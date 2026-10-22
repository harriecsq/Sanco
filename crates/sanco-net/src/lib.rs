//! Peer-to-peer sync over Tailscale: the HTTP server every PC runs, the
//! client that pulls from peers, and the loops that keep them in step.
//! Database logic stays in `sanco-core`; this crate only moves its data.

pub mod client;
pub mod error;
pub mod joincode;
pub mod node;
pub mod server;
pub mod tailscale;
pub mod token;
pub mod update;

pub use error::{Error, Result};
pub use joincode::JoinCode;
pub use update::{UpdateInfo, UpdateOffer};
pub use node::{
    DEFAULT_PORT, EventFn, LocalChanges, Node, NodeConfig, NodeEvent, PeerState, PeerStatus, join,
};
