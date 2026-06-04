//! Presentation layer: Bevy systems that map simulation state to visual transforms and gizmos.
//!
//! This module handles the visual representation of simulation entities,
//! separate from the GUI (egui panels) and the simulation logic itself.

mod bodies;
mod body_material;
mod body_mesh;
mod lights;
mod labels;
mod rotation;
mod trajectory;
mod trajectory_material;

pub use bodies::position_bodies;
pub use body_material::{BodyWireframeMaterial, BodyWireframeMaterialPlugin};
pub use body_mesh::{
    spawn_body_wireframe_meshes,
    spawn_terminator_meshes,
    update_terminator_meshes,
    cleanup_orphaned_body_wireframes,
    BodyWireframeMesh,
    BodyWireframeLink,
    TerminatorMesh,
    TerminatorLinks,
};
pub use lights::adjust_lights;
pub use labels::label_bodies;
pub use rotation::orient_bodies;
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
