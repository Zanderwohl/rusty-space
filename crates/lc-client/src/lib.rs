//! The Lightcone client.
//!
//! Single process: no server, no network. The parts that decide *what* to draw are plain
//! functions with no engine in them, and the Bevy wiring sits on top — phase 5 found that a
//! renderer made of ECS systems cannot be reused, so this one keeps the decisions separable
//! from the drawing.

// Moved into `lc-world`, because the server is authoritative over ship motion and has to run
// the same code the client predicts with. Re-exported so the client's own paths still read the
// way they did.
pub use lc_world::{coast, flight, navigation, system};

pub mod action;
#[cfg(not(target_arch = "wasm32"))]
pub mod auth;
#[cfg(not(target_arch = "wasm32"))]
pub mod broker;
pub mod app;
pub mod bookshelf;
pub mod chat;
pub mod curve;
pub mod demos;
pub mod dev;
pub mod entry;
pub mod envelope;
pub mod faces;
pub mod hud;
pub mod hull;
pub mod input;
pub mod library;
pub mod link;
pub mod map;
pub mod map_panel;
pub mod map_pick;
pub mod map_source;
#[cfg(not(target_arch = "wasm32"))]
pub mod local;
pub mod menu;
pub mod panels;
pub mod pick;
pub mod plume;
pub mod plot;
pub mod radio_panel;
pub mod range;
pub mod reader;
pub mod refit_panel;
pub mod resolved;
pub mod session;
#[cfg(not(target_arch = "wasm32"))]
pub mod signin_ui;
pub mod sky_asset;
pub mod starfield;
pub mod telescope_panel;
pub mod tonemap;
pub mod ui;
pub mod uplink;
#[cfg(not(target_arch = "wasm32"))]
pub mod vault;
pub mod view;
pub mod watch;

/// The game ticket this client will present when it opens a socket.
///
/// Where it comes from differs by build and the ticket does not: the browser reads it off the
/// launching page, and the desktop trades a device grant for one. That is the point of
/// `lightcone/docs/16-identity.md`'s ticket — the server has one code path and no notion of
/// which build it is talking to.
///
/// `None` on a deployment with no sign-in, which is the development and offline case.
#[derive(bevy::prelude::Resource, Default)]
pub struct Ticket(pub Option<String>);
