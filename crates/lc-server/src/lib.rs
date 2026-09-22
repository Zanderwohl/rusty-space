//! The Lightcone server.
//!
//! Server authoritative, without exception. Clients send intents; the server decides what
//! happened, when it happened, and who is entitled to know. The last of those is the whole
//! deliverable — see `lightcone/docs/08-networking.md` — and it lives in `lc-proto`, because
//! the type the event channel carries is the gate.
//!
//! Nothing here is optimized. The filter's correctness is what is being built.

#![forbid(unsafe_code)]
// Nothing in the game loop panics: startup may, and past it a wire message, a row or another
// craft's report is data. Every exception carries an `allow` with its reason.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic))]

pub mod ability;
pub mod archive;
pub mod admin;
pub mod chase;
pub mod director;
pub mod drive;
pub mod fitting;
pub(crate) mod instruments;
pub mod journal;
pub mod library;
pub mod persist;
pub mod radio;
pub mod rate;
pub mod server;
pub mod status;
pub mod systems;
#[cfg(test)]
pub mod testing;
pub mod ticket;
pub mod transport;
pub mod websocket;
pub mod world;
