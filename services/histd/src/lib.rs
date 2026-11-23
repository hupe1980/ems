//! Fleet historian for settlement registers and § 14a control evidence.
//!
//! PostgreSQL stores exact decimal register values. Each assertion identifies a
//! producer revision of a site and quarter hour. The historian records its own
//! knowledge time so a late correction leaves earlier settlements reproducible.
//!
//! The separate control-event record includes the requested ceiling, the response
//! and the connection-point trace `[A1 7.2]`. Its retention deadline follows
//! `[A1 7.3]`. Gateways keep a local copy and forward it when connected.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    // Domain nouns — MiSpeL, SQLite, PostgreSQL, ENTSO-E, ICON-D2 — are
    // capitalised because that is how they are spelled, not because they are
    // identifiers. The same allowance the domain crates carry.
    clippy::doc_markdown,
    clippy::similar_names
)]

pub mod api;
pub mod config;
pub mod export;
pub mod mcp_server;
mod registers;
pub mod store;

pub use config::Settings;
pub use store::{Store, StoreError};
