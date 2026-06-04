//! Presentation layer: Bevy systems that map simulation state to visual transforms and gizmos.
//!
//! This module handles the visual representation of simulation entities,
//! separate from the GUI (egui panels) and the simulation logic itself.

mod bodies;
mod body_material;
mod body_mesh;
mod body_point;
mod body_point_material;
mod lights;
mod labels;
mod rotation;
mod trajectory;
mod trajectory_material;

pub use bodies::position_bodies;
pub use body_material::{BodyWireframeMaterial, BodyWireframeMaterialPlugin, OccluderMaterial, OccluderMaterialPlugin};
pub use body_mesh::{
    spawn_body_wireframe_meshes,
    spawn_body_occluders,
    spawn_terminator_meshes,
    update_terminator_meshes,
    update_wireframe_lighting,
    update_occluder_lighting,
    update_wireframe_thickness,
    update_occluder_scale,
    cleanup_orphaned_body_wireframes,
    BodyWireframeMesh,
    BodyWireframeLink,
    OccluderMesh,
    OccluderLink,
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
pub use body_point::{
    spawn_body_point_meshes,
    update_body_points,
    cleanup_orphaned_body_points,
    BodyPointMesh,
    BodyPointLink,
};
pub use body_point_material::{BodyPointMaterial, BodyPointMaterialPlugin};
