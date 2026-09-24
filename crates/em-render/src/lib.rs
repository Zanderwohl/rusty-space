//! Reusable Bevy rendering for orbital scenes.
//!
//! Shared by both products, so nothing here may know a game or TTRPG rule.
//!
//! Shader asset paths resolve against the *host application's* asset directory, because the
//! host owns `AssetPlugin`. A consumer of this crate supplies the `.wgsl` files the materials
//! name.

#![forbid(unsafe_code)]

pub mod atmosphere_material;
pub mod body_material;
pub mod body_surface_material;
pub mod body_point_material;
pub mod drone_material;
pub mod encounter_marker_material;
pub mod field_material;
pub mod hull_material;
pub mod local_starfield_material;
pub mod plume_material;
pub mod population_material;
pub mod relativistic_starfield_material;
pub mod render_space;
pub mod wire_mesh;

pub use render_space::{ToRender, render_to_sim, sim_to_render};
