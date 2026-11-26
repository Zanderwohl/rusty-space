// A planet's rings: a flat annulus, with the bands evaluated per fragment.
//
// The mesh only has to cover the annulus. Where the material is, and how deep, comes from the
// band list in the uniform, box-filtered over the pixel's radial footprint: an edge is sharp at
// any zoom, and a ringlet or gap narrower than a pixel dims or brightens that pixel by its share
// of it rather than appearing or vanishing with the sample position.
//
// Per-vertex:
//   POSITION : the annulus in its own frame, unit outer radius, centered on the planet
//   NORMAL   : the ring's pole

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) local_position: vec3<f32>,
    @location(2) normal: vec3<f32>,
}

/// Must match `em_render::ring_material::RING_BANDS`.
const RING_BANDS: u32 = 16u;

/// Grains in one repeat of `grain_volume`. Must match `em_render::population_material::GRAIN_TILE`.
const GRAIN_TILE: f32 = 16.0;

/// The grain's mean, which it is faded toward where it is finer than a pixel.
const GRAIN_MEAN: f32 = 0.5;

struct RingUniform {
    tint: vec4<f32>,
    bands: array<vec4<f32>, RING_BANDS>,
    params: vec4<f32>,
    grain: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: RingUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var grain_volume: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var grain_sampler: sampler;

/// Depth over the deepest band's, averaged over the radii `radius +- width / 2`.
///
/// Each band adds its depth times the share of the window it covers. The window is the pixel's
/// footprint along the radius, so this is the box filter of the radial profile.
///
/// Edges are taken relative to `radius` before clamping to the window. Near the outer edge a
/// close pixel is a millionth of the radius wide, and offsetting the radius by half of that first
/// loses most of its bits.
fn depth_over(radius: f32, width: f32) -> f32 {
    let half = 0.5 * width;
    let count = min(u32(material.params.x), RING_BANDS);
    var sum = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let band = material.bands[i];
        sum = sum + band.z * (clamp(band.y - radius, -half, half) - clamp(band.x - radius, -half, half));
    }
    return sum / width;
}

/// The grain at `radius`, in `0..1`, faded toward its mean where a grain is finer than `width`:
/// past that it would alias into moire rather than read as ringlets.
fn grain_at(radius: f32, width: f32) -> f32 {
    let frequency = material.grain.x;
    let seed = material.grain.z;
    let at = vec3<f32>(radius * frequency + seed, seed * 2.7, seed * 1.3);
    let sampled = textureSampleLevel(grain_volume, grain_sampler, at / GRAIN_TILE, 0.0).r;
    let resolved = clamp(1.0 / max(2.0 * width * frequency, 1e-6) - 0.5, 0.0, 1.0);
    return mix(GRAIN_MEAN, sampled, resolved);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(
        world_from_local, vec4<f32>(vertex.position, 1.0));
    out.world_position = world.xyz;
    out.clip_position = position_world_to_clip(world.xyz);
    out.local_position = vertex.position;
    out.normal = normalize(mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index));
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let radius = length(in.local_position);
    // Floored, because a sightline along the sheet has a footprint of the whole ring and one
    // straight down a tangent has none.
    let width = max(fwidth(radius), 1e-6);
    let depth = depth_over(radius, width);
    if (depth <= 0.0) {
        discard;
    }

    let to_camera = normalize(view.world_position.xyz - in.world_position);
    let facing = abs(dot(in.normal, to_camera));
    let limb = 1.0 + material.params.z * (1.0 / max(facing, 0.08) - 1.0);
    let speckle = mix(1.0, grain_at(radius, width), material.grain.y);

    // `1 - (1 - t)^limb`, not `t * limb`: a sightline along the sheet has unbounded path length,
    // and a linear law painted edge-on rings at twice full white.
    let single = clamp(depth * material.params.y * speckle, 0.0, 1.0);
    let alpha = 1.0 - pow(max(1.0 - single, 0.0), limb);
    // Premultiplied and additive: alpha leaves as zero and the color carries it.
    return vec4<f32>(material.tint.rgb * alpha, 0.0);
}
