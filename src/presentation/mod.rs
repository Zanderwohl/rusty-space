//! Presentation layer: Bevy systems that map simulation state to visual transforms and gizmos.
//!
//! This module handles the visual representation of simulation entities,
//! separate from the GUI (egui panels) and the simulation logic itself.

mod body_material;
mod body_mesh;
mod body_point;
mod body_point_material;
mod celestial_markers;
mod lights;
pub mod render_space;

mod labels;
mod local_starfield;
mod local_starfield_material;
mod rotation;
mod soi;
mod soi_material;
mod star_cache;
mod starfield;
mod starfield_material;
mod trajectory;
mod trajectory_material;

pub use body_material::{BodyWireframeMaterial, BodyWireframeMaterialPlugin, OccluderMaterial, OccluderMaterialPlugin, BASE_TUBE_RADIUS};
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
pub use rotation::render_axes;
pub use labels::label_bodies;
pub use local_starfield::{spawn_local_starfield, update_local_starfield, clear_local_starfield, LocalStarfield};
pub use local_starfield_material::{LocalStarfieldMaterial, LocalStarfieldMaterialPlugin};
pub use trajectory::{
    build_trajectory_meshes,
    build_working_trajectory_points,
    calculate_tube_radius,
    cleanup_orphaned_trajectory_meshes,
    draw_trajectory_marker_labels,
    update_focused_trajectory_markers,
    update_mouse_hit_marker,
    update_trajectory_material_brightness,
    rebuild_trajectory_caches,
    refresh_precessing_trajectories,
    spawn_trajectory_mesh,
    spawn_trajectory_meshes_for_bodies,
    FocusedTrajectoryMarkerKind,
    FocusedTrajectoryMarker,
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
pub use soi::{
    spawn_soi_meshes,
    update_soi_shells,
    cleanup_orphaned_soi_meshes,
    SoiMeshes,
    SoiPointsMesh,
    SoiRingMesh,
    SoiLink,
};
pub use soi_material::{
    SoiPointsMaterial,
    SoiPointsMaterialPlugin,
    SoiRingMaterial,
    SoiRingMaterialPlugin,
    SoiShapeUniform,
};
pub use celestial_markers::{
    sync_celestial_markers,
    update_celestial_markers,
    cleanup_celestial_markers,
    CelestialMarker,
    CelestialMarkerCache,
};
pub use star_cache::{build_star_lighting_cache, CachedStarData, StarLightingFrameCache};
pub use starfield::{spawn_starfield, update_starfield_brightness, Starfield};
pub use starfield_material::{StarfieldMaterial, StarfieldMaterialPlugin};
