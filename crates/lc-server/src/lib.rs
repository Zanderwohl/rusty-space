//! The Lightcone server.
//!
//! Server authoritative, without exception. Clients send intents; the server decides what
//! happened, when it happened, and who is entitled to know. The last of those is the whole
//! deliverable — see `lightcone/docs/08-networking.md` — and it lives in `lc-proto`, because
//! the type the event channel carries is the gate.
//!
//! Nothing here is optimized. The filter's correctness is what is being built.
//!
//! What outlives the process — checkpoints, knowledge, presets, the Postgres journal and the
//! admin routes over them — is behind the `storage` feature, on by default and off for the
//! client's single-player shard, which never saves.

#![forbid(unsafe_code)]
// Nothing in the game loop panics: startup may, and past it a wire message, a row or another
// craft's report is data. Every exception carries an `allow` with its reason.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic))]

pub mod ability;
#[cfg(feature = "storage")]
pub mod archive;
#[cfg(feature = "storage")]
pub mod admin;
#[cfg(feature = "storage")]
pub(crate) mod cbor;
pub mod chase;
pub mod command;
pub mod director;
pub mod drive;
pub mod emit;
pub mod field;
pub(crate) mod fits;
pub mod fitting;
pub(crate) mod instruments;
pub mod journal;
pub mod library;
pub mod park;
#[cfg(feature = "storage")]
pub mod persist;
pub mod planets;
pub mod presets;
pub mod radio;
pub mod rate;
pub mod server;
#[cfg(feature = "storage")]
pub mod status;
#[cfg(feature = "storage")]
pub mod systems;
#[cfg(test)]
pub mod testing;
pub mod ticket;
pub mod timing;
pub mod transport;
pub mod websocket;
pub mod world;
