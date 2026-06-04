//! Presentation layer: Bevy systems that map simulation state to visual transforms and gizmos.
//!
//! This module handles the visual representation of simulation entities,
//! separate from the GUI (egui panels) and the simulation logic itself.

mod bodies;
mod lights;
mod labels;
mod rotation;
mod trajectory;
mod trajectory_material;

pub use bodies::position_bodies;
pub use lights::adjust_lights;
pub use labels::label_bodies;
pub use rotation::{orient_bodies, render_axes};
pub use trajectory::{
    build_trajectory_meshes,
    cleanup_orphaned_trajectory_meshes,
    rebuild_trajectory_caches,
    refresh_precessing_trajectories,
    spawn_trajectory_mesh,
    spawn_trajectory_meshes_for_bodies,
    TrajectoryCache,
    TrajectoryMesh,
    TrajectoryMeshLink,
};
pub use trajectory_material::{TrajectoryMaterial, TrajectoryMaterialPlugin};
