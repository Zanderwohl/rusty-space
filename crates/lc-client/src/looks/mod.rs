//! Ship surfaces in look development: no part kind draws one yet.
//!
//! Each maps a box face in its own frame, and a curved mesh by the face each point faces most, as
//! a cube map would; a box is where they look as meant. Each turns its star about world Y at its
//! own rate from `globals.time`, never from a uniform written each frame; a rate whose period
//! divides an hour turns seamlessly through the hourly wrap of that clock.

pub mod aztec;
pub mod kinds;
pub mod obsidian;
pub mod outline;
pub mod racks;
pub mod sprue;

use bevy::prelude::*;

pub struct LooksPlugin;

impl Plugin for LooksPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MaterialPlugin::<aztec::AztecMaterial>::default(),
            MaterialPlugin::<sprue::SprueMaterial>::default(),
            MaterialPlugin::<obsidian::ObsidianMaterial>::default(),
            MaterialPlugin::<racks::RacksMaterial>::default(),
        ))
        .add_systems(PostUpdate, outline::outline_roofs.after(bevy::transform::TransformSystems::Propagate));
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
