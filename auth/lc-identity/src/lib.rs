//! The Lightcone identity broker: upstream providers in, opaque account ids out.
//!
//! A library with a thin binary over it, the shape `lc-server` uses, so the parts worth testing
//! are reachable without a socket.
//!
//! It is an OAuth2 *client* facing upstream and deliberately much less than an OAuth2 server
//! facing the site. `lightcone/docs/16-identity.md` explains why that asymmetry is the design
//! rather than a shortcut, and what it gives up.

#![forbid(unsafe_code)]

pub mod ability;
pub mod actions;
pub mod assets;
pub mod attempts;
pub mod bans;
pub mod config;
pub mod level;
pub mod pages;
pub mod password;
pub mod providers;
pub mod routes;
pub mod schema;
pub mod signin;
pub mod store;
pub mod ticket;
pub mod upstream;
