//! The shadow a hull casts on itself from its star: each hull that wants one has a depth map of
//! its own mesh seen from the star, in the ship's frame and meters, drawn afresh every frame
//! before any camera, and the hull material compares against it. See
//! `lightcone/docs/32-ship-rendering.md` §Shadows.
//!
//! Not Bevy's directional shadows: those are cascades fit to each camera's frustum, in render
//! units of an astronomical unit, where a ship is a billionth of one across. Here the map is fit
//! to the hull alone and is the same for every camera.
//!
//! The map's frame is [`light_basis`] of the star's direction in the ship's frame, about the
//! mesh's bounds; `hull.wgsl` builds the same frame from the same uniform, so the two never
//! disagree about where a texel is. A hull's material must be its own: the map and the bounds are
//! written into it.

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::schedule::camera_driver;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::camera::primitives::MeshAabb;
use bevy::render::mesh::{MeshVertexBufferLayoutRef, RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::uniform_buffer;
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId, CompareFunction,
    DepthBiasState, DepthStencilState, DynamicUniformBuffer, Extent3d, LoadOp, MultisampleState, Operations, PipelineCache,
    PrimitiveState, RenderPassDepthStencilAttachment, RenderPassDescriptor, RenderPipelineDescriptor, ShaderStages, ShaderType,
    SpecializedMeshPipeline, SpecializedMeshPipelineError, SpecializedMeshPipelines, StencilState, StoreOp, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages, VertexState,
};
use bevy::render::renderer::{PendingCommandBuffers, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::transform::TransformSystems;
use em_render::hull_material::HullMaterial;

use crate::hull_mesh::{HullMeshSystems, HullPixels};

/// Texels along a map's side.
pub const MAP_TEXELS: u32 = 2048;
/// A hull this many pixels long or more is given a map, and keeps it until it falls under
/// [`RELEASE_PX`]: the gap keeps one held near either from being given and taken in turn.
const GIVE_PX: f32 = 128.0;
const RELEASE_PX: f32 = 32.0;

/// Asks for a hull's shadow. On the entity holding its `Mesh3d` and its own
/// `MeshMaterial3d<HullMaterial>`.
#[derive(Component, Default)]
pub struct HullShadow {
    /// Shadowed at any size, as a void wants.
    pub pinned: bool,
    map: Option<Handle<Image>>,
}

impl HullShadow {
    pub fn pinned() -> Self {
        HullShadow { pinned: true, map: None }
    }
}

pub struct HullShadowPlugin;

impl Plugin for HullShadowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShadowJobs>()
            .add_plugins(ExtractResourcePlugin::<ShadowJobs>::default())
            .add_systems(PostUpdate, (fit_maps.after(HullMeshSystems), aim.after(TransformSystems::Propagate)).chain());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<SpecializedMeshPipelines<CasterPipeline>>()
            .init_resource::<Prepared>()
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareBindGroups))
            .add_systems(RenderGraph, cast.in_set(RenderGraphSystems::Render).before(camera_driver));
    }

    fn finish(&self, app: &mut App) {
        let shader = app.world().resource::<AssetServer>().load("shaders/hull_shadow.wgsl");
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.insert_resource(CasterPipeline {
            layout: BindGroupLayoutDescriptor::new(
                "hull_shadow_caster",
                &BindGroupLayoutEntries::single(ShaderStages::VERTEX, uniform_buffer::<Caster>(true)),
            ),
            shader,
        });
    }
}

/// The frame a map is drawn in, as columns: x and y across the map, and the direction to the star,
/// all in the ship's frame. `hull.wgsl`'s `light_basis` is the same.
pub fn light_basis(to_star: Vec3) -> Mat3 {
    let s = to_star.normalize_or(Vec3::Z);
    let helper = if s.z.abs() > 0.9 { Vec3::X } else { Vec3::Z };
    let x = helper.cross(s).normalize();
    Mat3::from_cols(x, s.cross(x), s)
}

/// Clip space of the map from the ship's frame, meters: `±1` across the sphere of `radius` about
/// `center`, and depth from 0 on the star's side of it to 1 on the far side.
pub fn light_from_ship(to_star: Vec3, center: Vec3, radius: f32) -> Mat4 {
    let Mat3 { x_axis: x, y_axis: y, z_axis: s } = light_basis(to_star);
    let row = |v: Vec3, w: f32| v.extend(w);
    Mat4::from_cols(
        row(x / radius, -x.dot(center) / radius),
        row(y / radius, -y.dot(center) / radius),
        row(-s / (2.0 * radius), (radius + s.dot(center)) / (2.0 * radius)),
        Vec4::W,
    )
    .transpose()
}

/// The sphere the map is fit to: the bounds' center and half their diagonal, in `xyz` and `w`, as
/// `HullUniform::shadow` holds it. Zero for a mesh with no positions.
pub fn bounds(mesh: &Mesh) -> Vec4 {
    let Some(aabb) = mesh.compute_aabb() else { return Vec4::ZERO };
    let half: Vec3 = aabb.half_extents.into();
    let center: Vec3 = aabb.center.into();
    center.extend(half.length().max(f32::MIN_POSITIVE))
}

fn new_map() -> Image {
    let size = Extent3d { width: MAP_TEXELS, height: MAP_TEXELS, depth_or_array_layers: 1 };
    let mut image = Image::default();
    image.data = None;
    image.texture_descriptor = TextureDescriptor {
        label: Some("hull_shadow_map"),
        size,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        mip_level_count: 1,
        sample_count: 1,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    };
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image
}

/// Gives a map to each hull big enough on screen, or pinned, and takes it from one too small.
/// Writes the material only when a map comes or goes or the mesh changes.
fn fit_maps(
    mut hulls: Query<(&mut HullShadow, &HullPixels, Option<Ref<Mesh3d>>, &MeshMaterial3d<HullMaterial>)>,
    mut images: ResMut<Assets<Image>>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<HullMaterial>>,
) {
    for (mut shadow, pixels, mesh, material) in &mut hulls {
        let wanted = mesh.is_some() && (shadow.pinned || pixels.0 >= GIVE_PX || (shadow.map.is_some() && pixels.0 >= RELEASE_PX));
        let fresh = wanted && shadow.map.is_none();
        if fresh {
            shadow.map = Some(images.add(new_map()));
        } else if !wanted && shadow.map.is_some() {
            shadow.map = None;
        }
        let remeshed = mesh.as_ref().is_some_and(|m| m.is_changed());
        let Some(mut asset) = (if fresh || remeshed || (!wanted && materials.get(&material.0).is_some_and(|m| m.shadow_map.is_some())) {
            materials.get_mut(&material.0)
        } else {
            None
        }) else {
            continue;
        };
        match (&shadow.map, mesh.and_then(|m| meshes.get(&m.0))) {
            (Some(map), Some(mesh)) => {
                asset.shadow_map = Some(map.clone());
                asset.uniforms.shadow = bounds(mesh);
            }
            _ => {
                asset.shadow_map = None;
                asset.uniforms.shadow = Vec4::ZERO;
            }
        }
    }
}

#[derive(Clone)]
struct Job {
    mesh: AssetId<Mesh>,
    map: AssetId<Image>,
    light_from_ship: Mat4,
}

/// Every map to draw this frame.
#[derive(Resource, Clone, Default, ExtractResource)]
struct ShadowJobs(Vec<Job>);

/// The star's direction in the ship's frame is taken as `hull.wgsl` takes it, from the material's
/// `to_star` through the hull's own transform, so map and lookup agree on the frame.
fn aim(
    hulls: Query<(&HullShadow, &Mesh3d, &MeshMaterial3d<HullMaterial>, &GlobalTransform, &InheritedVisibility)>,
    materials: Res<Assets<HullMaterial>>,
    mut jobs: ResMut<ShadowJobs>,
) {
    jobs.0.clear();
    for (shadow, mesh, material, transform, visible) in &hulls {
        let (Some(map), Some(material), true) = (&shadow.map, materials.get(&material.0), visible.get()) else { continue };
        let sphere = material.uniforms.shadow;
        if sphere.w <= 0.0 || material.shadow_map.as_ref() != Some(map) {
            continue;
        }
        let to_star = transform.affine().matrix3.transpose() * Vec3A::from(material.uniforms.to_star.truncate());
        jobs.0.push(Job { mesh: mesh.id(), map: map.id(), light_from_ship: light_from_ship(to_star.into(), sphere.truncate(), sphere.w) });
    }
}

#[derive(Clone, Copy, ShaderType)]
struct Caster {
    light_from_ship: Mat4,
}

#[derive(Resource)]
struct CasterPipeline {
    layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

impl SpecializedMeshPipeline for CasterPipeline {
    type Key = ();

    fn specialize(&self, _key: (), layout: &MeshVertexBufferLayoutRef) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        Ok(RenderPipelineDescriptor {
            label: Some("hull_shadow_caster".into()),
            layout: vec![self.layout.clone()],
            immediate_size: 0,
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs: vec![],
                entry_point: Some("vertex".into()),
                buffers: vec![layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?],
            },
            // Both faces: a bay's cards are two-sided and its shell's walls thin.
            primitive: PrimitiveState { cull_mode: None, ..default() },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Less),
                stencil: StencilState::default(),
                bias: DepthBiasState { constant: 0, slope_scale: 1.5, clamp: 0.0 },
            }),
            multisample: MultisampleState::default(),
            fragment: None,
            zero_initialize_workgroup_memory: false,
        })
    }
}

struct Draw {
    pipeline: CachedRenderPipelineId,
    mesh: AssetId<Mesh>,
    map: AssetId<Image>,
    offset: u32,
}

#[derive(Resource, Default)]
struct Prepared {
    draws: Vec<Draw>,
    uniforms: DynamicUniformBuffer<Caster>,
    bind_group: Option<BindGroup>,
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    jobs: Res<ShadowJobs>,
    caster: Res<CasterPipeline>,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedMeshPipelines<CasterPipeline>>,
    meshes: Res<RenderAssets<RenderMesh>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut prepared: ResMut<Prepared>,
) {
    let prepared = &mut *prepared;
    prepared.draws.clear();
    prepared.uniforms.clear();
    prepared.bind_group = None;
    for job in &jobs.0 {
        let Some(mesh) = meshes.get(job.mesh) else { continue };
        let Ok(pipeline) = pipelines.specialize(&cache, &caster, (), &mesh.layout) else { continue };
        let offset = prepared.uniforms.push(&Caster { light_from_ship: job.light_from_ship });
        prepared.draws.push(Draw { pipeline, mesh: job.mesh, map: job.map, offset });
    }
    if prepared.draws.is_empty() {
        return;
    }
    prepared.uniforms.write_buffer(&device, &queue);
    if let Some(binding) = prepared.uniforms.binding() {
        let layout = cache.get_bind_group_layout(&caster.layout);
        prepared.bind_group = Some(device.create_bind_group("hull_shadow_caster", &layout, &BindGroupEntries::single(binding)));
    }
}

/// Each map cleared and its hull drawn into it, ahead of every camera's command buffers.
fn cast(
    prepared: Res<Prepared>,
    cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    allocator: Res<MeshAllocator>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    mut pending: ResMut<PendingCommandBuffers>,
) {
    let Some(bind_group) = &prepared.bind_group else { return };
    let mut encoder = device.create_command_encoder(&Default::default());
    for draw in &prepared.draws {
        let (Some(pipeline), Some(mesh), Some(map), Some(vertices)) = (
            cache.get_render_pipeline(draw.pipeline),
            meshes.get(draw.mesh),
            images.get(draw.map),
            allocator.mesh_vertex_slice(&draw.mesh),
        ) else {
            continue;
        };
        let RenderMeshBufferInfo::Indexed { index_format, .. } = mesh.buffer_info else { continue };
        let Some(indices) = allocator.mesh_index_slice(&draw.mesh) else { continue };
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("hull_shadow"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &map.texture_view,
                depth_ops: Some(Operations { load: LoadOp::Clear(1.0), store: StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[draw.offset]);
        pass.set_vertex_buffer(0, *vertices.buffer.slice(..));
        pass.set_index_buffer(*indices.buffer.slice(..), index_format);
        pass.draw_indexed(indices.range.clone(), vertices.range.start as i32, 0..1);
    }
    pending.push_encoder(encoder);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_basis_is_a_rotation_with_the_star_along_its_third_axis() {
        for star in [Vec3::X, Vec3::new(0.2, -0.7, 0.4), Vec3::Z, Vec3::new(0.01, 0.0, -1.0)] {
            let m = light_basis(star);
            assert!((m.determinant() - 1.0).abs() < 1e-5, "{star}");
            assert!(m.z_axis.abs_diff_eq(star.normalize(), 1e-6));
            assert!((m.transpose() * m).abs_diff_eq(Mat3::IDENTITY, 1e-5));
        }
    }

    /// The sphere's point toward the star is at depth 0 and the far point at 1, its rim at the
    /// edge of the map, and both in front of the clip plane.
    #[test]
    fn the_sphere_fills_the_map_and_its_depth() {
        let (star, center, radius) = (Vec3::new(0.3, 0.9, -0.2).normalize(), Vec3::new(4.0, -2.0, 7.0), 50.0);
        let m = light_from_ship(star, center, radius);
        let clip = |p: Vec3| m * p.extend(1.0);
        assert!((clip(center + star * radius).z).abs() < 1e-5);
        assert!((clip(center - star * radius).z - 1.0).abs() < 1e-5);
        let x = light_basis(star).x_axis;
        assert!((clip(center + x * radius).x - 1.0).abs() < 1e-5);
        assert!(clip(center).truncate().abs_diff_eq(Vec3::new(0.0, 0.0, 0.5), 1e-5));
    }
}
