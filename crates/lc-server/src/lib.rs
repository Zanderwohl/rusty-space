//! The Lightcone server.
//!
//! Server authoritative, without exception. Clients send intents; the server decides what
//! happened, when it happened, and who is entitled to know. The last of those is the whole
//! deliverable — see `lightcone/docs/08-networking.md` — and it lives in `lc-proto`, because
//! the type the event channel carries is the gate.
//!
//! Nothing here is optimised. The filter's correctness is what is being built.

#![forbid(unsafe_code)]

pub mod chase;
pub mod director;
pub mod drive;
pub mod journal;
pub mod library;
pub mod persist;
pub mod rate;
pub mod server;
#[cfg(test)]
pub mod testing;
pub mod ticket;
pub mod transport;
pub mod websocket;
pub mod world;
