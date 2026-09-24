//! Drones as stateless particles.
//!
//! Every drone's position is a closed form of its index, [`DroneUniform::seed`] and
//! [`DroneUniform::time`], evaluated in the vertex shader. Nothing per drone lives on the CPU, so
//! the picture at `t` is the same whenever and however often it is drawn, and there is no particle
//! simulation to keep in step — which is also why this is not `bevy_hanabi`, which the WebGPU
//! build would have to carry too. See `lightcone/docs/32-ship-rendering.md` §Drones.
//!
//! The material knows docks, targets, a patrol shape and a clock. What those mean is the host's.
//!
//! One mesh from [`drone_quads`] holds a quad per drone, each carrying its drone's index. The
//! index rides in the vertex rather than coming from `vertex_index`, because Bevy packs meshes
//! into shared buffers and `vertex_index` includes the mesh's offset there. The mesh's positions
//! are not where anything is drawn, so its entity needs `NoFrustumCulling`.

use bevy::asset::RenderAssetUsages;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};

/// Docks the uniform holds. Must match `MAX_DOCKS` in `drones.wgsl`.
pub const MAX_DOCKS: usize = 16;
/// Targets the uniform holds. Must match `MAX_TARGETS` in `drones.wgsl`.
pub const MAX_TARGETS: usize = 64;

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct DroneUniform {
    /// Where drones leave from and return to, in the mesh's local frame. `w` unused.
    pub docks: [Vec4; MAX_DOCKS],
    /// Where working drones go, in the mesh's local frame. `w` unused.
    pub targets: [Vec4; MAX_TARGETS],
    /// Center of the patrol ellipsoid, local frame. `w` unused.
    pub patrol_center: Vec4,
    /// Semi-axes of the patrol ellipsoid, local frame. `w` unused. Also what arcs bow away
    /// from: an arc lifts along the direction from the center to its chord's midpoint.
    pub patrol_radii: Vec4,
    /// Linear color of a mote, premultiplied by its brightness.
    pub color: Vec4,
    /// Color a carrying drone adds on its way home.
    pub carry_color: Vec4,
    /// Seconds. Kept small by the host — it is `f32` in the shader, so a clock of a day
    /// resolves only to about eight milliseconds.
    pub time: f32,
    pub seed: u32,
    /// Drones drawn, at most the quads in the mesh; the rest collapse to nothing.
    pub count: u32,
    pub dock_count: u32,
    pub target_count: u32,
    /// Fraction of drones working, `0..1`. Chosen by a hash of the index, so raising it adds
    /// drones to the work without reshuffling the ones already on it.
    pub working: f32,
    /// Fraction patrolling, `0..1`, taken from the drones not working. The rest are docked and
    /// not drawn.
    pub patrol: f32,
    /// `0..1`: how brightly a returning drone glows in `carry_color`. One on a dismantle.
    pub carrying: f32,
    /// Mean seconds for one trip out, dwell and back. Each drone's own is within a quarter of it.
    pub cycle_s: f32,
    /// Fraction of a trip spent at the target.
    pub dwell: f32,
    /// How far an arc bows out, as a fraction of its chord.
    pub arc_lift: f32,
    /// Meters: how far a drone wanders about its target while it dwells.
    pub hover_m: f32,
    /// Seconds for a patroller to circle the ellipsoid once, on average.
    pub patrol_period_s: f32,
    /// Meters across a mote.
    pub mote_m: f32,
    /// Meters across a mote once it has become haze. About the spacing of neighboring drones,
    /// so the haze is smooth rather than a field of blobs.
    pub haze_m: f32,
    /// Pixels across at which a mote starts turning into haze. It is haze entirely by a third
    /// of this, which is above a pixel for any value from three up.
    pub haze_px: f32,
}

impl Default for DroneUniform {
    fn default() -> Self {
        Self {
            docks: [Vec4::ZERO; MAX_DOCKS],
            targets: [Vec4::ZERO; MAX_TARGETS],
            patrol_center: Vec4::ZERO,
            patrol_radii: Vec4::ONE,
            color: Vec4::new(0.9, 0.95, 1.0, 1.0),
            carry_color: Vec4::new(2.0, 0.9, 0.3, 1.0),
            time: 0.0,
            seed: 0,
            count: 0,
            dock_count: 0,
            target_count: 0,
            working: 0.0,
            patrol: 0.05,
            carrying: 0.0,
            cycle_s: 12.0,
            dwell: 0.3,
            arc_lift: 0.35,
            hover_m: 0.5,
            patrol_period_s: 90.0,
            mote_m: 0.4,
            haze_m: 6.0,
            haze_px: 3.0,
        }
    }
}

impl DroneUniform {
    /// Writes as many docks as fit and says how many there are.
    pub fn set_docks(&mut self, docks: &[Vec3]) {
        self.dock_count = fill(&mut self.docks, docks);
    }

    /// Writes as many targets as fit and says how many there are.
    pub fn set_targets(&mut self, targets: &[Vec3]) {
        self.target_count = fill(&mut self.targets, targets);
    }
}

fn fill(slots: &mut [Vec4], points: &[Vec3]) -> u32 {
    let n = points.len().min(slots.len());
    for (slot, point) in slots.iter_mut().zip(points) {
        *slot = point.extend(0.0);
    }
    n as u32
}

/// One quad per drone, `capacity` of them. Corners in `xy`, the drone's index in `z`: exact in
/// `f32` up to sixteen million drones.
pub fn drone_quads(capacity: u32) -> Mesh {
    const CORNERS: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
    let mut positions = Vec::with_capacity(capacity as usize * 4);
    let mut indices = Vec::with_capacity(capacity as usize * 6);
    for i in 0..capacity {
        for [x, y] in CORNERS {
            positions.push([x, y, i as f32]);
        }
        let base = i * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_indices(Indices::U32(indices))
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct DroneMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: DroneUniform,
}

impl Material for DroneMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/drones.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/drones.wgsl".into()
    }

    /// Additive, so the fragment returns alpha zero: `AlphaMode::Add` is premultiplied.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    /// The mesh's positions are not positions, so the default prepass and shadow shaders would
    /// draw the quads stacked at the origin.
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

pub struct DroneMaterialPlugin;

impl Plugin for DroneMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<DroneMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_quad_carries_its_own_index() {
        let mesh = drone_quads(3);
        let Some(bevy_mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions are Float32x3");
        };
        let indices: Vec<f32> = positions.iter().map(|p| p[2]).collect();
        assert_eq!(indices, [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0]);
        assert_eq!(mesh.indices().map(|i| i.len()), Some(18));
    }

    #[test]
    fn fixtures_past_the_capacity_are_dropped_and_counted_out() {
        let mut uniform = DroneUniform::default();
        uniform.set_docks(&vec![Vec3::X; MAX_DOCKS + 5]);
        assert_eq!(uniform.dock_count as usize, MAX_DOCKS);
        uniform.set_targets(&[Vec3::Y, Vec3::Z]);
        assert_eq!(uniform.target_count, 2);
        assert_eq!(uniform.targets[1], Vec4::new(0.0, 0.0, 1.0, 0.0));
    }
}
