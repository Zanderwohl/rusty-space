//! Ship surfaces in look development: no part kind draws one yet.
//!
//! The sprue, racks and tubes are laid out in a part's own charts ([`crate::charts`]), so they
//! draw only on the hull mesher's meshes, and fit any primitive. The aztec and obsidian still map
//! a box face in its own frame, and a curved mesh by the face each point faces most, as a cube map
//! would; a box is where they look as meant. Each turns its star about world Y at its
//! own rate from `globals.time`, never from a uniform written each frame; a rate whose period
//! divides an hour turns seamlessly through the hourly wrap of that clock.

pub mod aztec;
pub mod kinds;
pub mod obsidian;
pub mod outline;
pub mod racks;
pub mod sprue;
pub mod tubes;

use bevy::prelude::*;

pub struct LooksPlugin;

impl Plugin for LooksPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MaterialPlugin::<aztec::AztecMaterial>::default(),
            MaterialPlugin::<sprue::SprueMaterial>::default(),
            MaterialPlugin::<obsidian::ObsidianMaterial>::default(),
            MaterialPlugin::<racks::RacksMaterial>::default(),
            MaterialPlugin::<tubes::TubesMaterial>::default(),
        ));
    }
}

/// Y-up, from elevation and yaw in degrees.
pub fn direction(elevation_deg: f32, yaw_deg: f32) -> Vec3 {
    let (e, y) = (elevation_deg.to_radians(), yaw_deg.to_radians());
    Vec3::new(e.cos() * y.cos(), e.sin(), e.cos() * y.sin())
}

/// The star's turn about world Y, rad/s, for one revolution in `period_s`; 0 holds it still.
pub fn spin(period_s: f32) -> f32 {
    if period_s > 0.0 { std::f32::consts::TAU / period_s } else { 0.0 }
}

/// Only position and normal: these shaders take nothing else from the mesh.
fn position_and_normal(
    descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
    layout: &bevy_mesh::MeshVertexBufferLayoutRef,
) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
    let vertex_layout = layout.0.get_layout(&[
        Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
        Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
    ])?;
    descriptor.vertex.buffers = vec![vertex_layout];
    Ok(())
}

/// Position, normal, and the part's own chart, its bounds and tangent: for a look laid out in a
/// part's charts, which only the hull mesher's meshes carry. See [`crate::charts`].
fn charted(
    descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
    layout: &bevy_mesh::MeshVertexBufferLayoutRef,
) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
    use em_render::hull_material::{ATTRIBUTE_HULL_CHART, ATTRIBUTE_HULL_CHART_BOUNDS};
    let vertex_layout = layout.0.get_layout(&[
        Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
        Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
        ATTRIBUTE_HULL_CHART.at_shader_location(2),
        ATTRIBUTE_HULL_CHART_BOUNDS.at_shader_location(3),
        Mesh::ATTRIBUTE_TANGENT.at_shader_location(4),
    ])?;
    descriptor.vertex.buffers = vec![vertex_layout];
    Ok(())
}
