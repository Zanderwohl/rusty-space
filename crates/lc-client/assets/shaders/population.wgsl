// A population's envelope: swarms, belts and clouds, drawn as a shell.
//
// A population has no members to draw -- that is the entire point of 04-stellar-photometry.md,
// which replaces the roster with a distribution. So the renderer draws the distribution: a
// shell at the population's orbital radius whose opacity at each latitude is the population's
// own sky density there. One shape covers every case. A belt's inclinations are narrow, so the
// density is a band near the plane and it reads as a ring; an isotropic swarm's density is flat
// and it reads as a sphere. Nothing special-cases either.
//
// Per-vertex:
//   POSITION : the surface in the population's own frame, with the transform scaling it
//   NORMAL   : the surface normal there -- its own direction for a shell, the pole for a ring
//   DENSITY  : density there, with the peak at one
//
// See lightcone/docs/07-rendering.md.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) density: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) local_direction: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) density: f32,
}

struct PopulationUniform {
    tint: vec4<f32>,
    /// Covering fraction, scaled for display. What the photometry says is there.
    opacity: f32,
    /// Grains per unit of the shell's own direction. Higher is finer.
    grain_frequency: f32,
    /// How much of the opacity the granularity carries.
    grain_strength: f32,
    /// Per-population, so two belts are not the same speckle.
    seed: f32,
    /// How much the limb brightens. A line of sight along a thin shell passes through far more
    /// of it than one straight through, which is why a ring has a bright edge.
    limb_gain: f32,
    /// What the shell is scaled to when the camera is inside it.
    ///
    /// From outside, a belt is a ring and the eye reads it as structure. From inside it covers
    /// the entire sky, and at the opacity that made the ring legible it becomes a grey wash
    /// that buries the star field. The same number cannot serve both, so the inside case is
    /// dimmed to what it is worth: a faint band, the way the zodiacal light is.
    inside_fade: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: PopulationUniform;

fn hash31(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn value_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let x00 = mix(hash31(i), hash31(i + vec3<f32>(1.0, 0.0, 0.0)), w.x);
    let x10 = mix(hash31(i + vec3<f32>(0.0, 1.0, 0.0)), hash31(i + vec3<f32>(1.0, 1.0, 0.0)), w.x);
    let x01 = mix(hash31(i + vec3<f32>(0.0, 0.0, 1.0)), hash31(i + vec3<f32>(1.0, 0.0, 1.0)), w.x);
    let x11 = mix(hash31(i + vec3<f32>(0.0, 1.0, 1.0)), hash31(i + vec3<f32>(1.0, 1.0, 1.0)), w.x);
    return mix(mix(x00, x10, w.y), mix(x01, x11, w.y), w.z);
}

/// Granularity: two octaves, to read as many small bodies rather than as a painted surface.
fn grain(direction: vec3<f32>, seed: f32) -> f32 {
    let p = direction * material.grain_frequency + vec3<f32>(seed, seed * 2.7, seed * 1.3);
    return value_noise(p) * 0.65 + value_noise(p * 2.9) * 0.35;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(
        world_from_local, vec4<f32>(vertex.position, 1.0));
    out.world_position = world.xyz;
    out.clip_position = position_world_to_clip(world.xyz);
    out.local_direction = normalize(vertex.position);
    out.normal = normalize(mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index));
    out.density = vertex.density;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if (in.density <= 0.0) {
        discard;
    }
    let to_camera = normalize(view.world_position.xyz - in.world_position);
    // A shell's normal is its own direction; a ring's is its pole. Passing it rather than
    // deriving it is what lets one shader draw both, and it is the whole of the difference: an
    // edge-on ring has a normal across the view and lights up, which is correct.
    let facing = abs(dot(in.normal, to_camera));
    let limb = 1.0 + material.limb_gain * (1.0 / max(facing, 0.08) - 1.0);

    let speckle = mix(1.0, grain(in.local_direction, material.seed), material.grain_strength);

    // What one straight-through pass covers, and then that pass repeated `limb` times over.
    //
    // `1 - (1 - t)^limb`, not `t * limb`. A sightline along a sheet has unbounded path length,
    // and under a linear law it paints unbounded light: edge-on, Saturn's rings came out at
    // twice full white. This is the same exponential `Population::absorbed_fraction` applies
    // to the star's light -- past a covering fraction of a few tenths the elements shadow one
    // another -- and it leaves the face-on case exactly as it was, because at `limb` of one it
    // is `t`. Only the grazing case changes, and only by being bounded.
    let single = clamp(in.density * material.opacity * speckle * material.inside_fade, 0.0, 1.0);
    let alpha = 1.0 - pow(max(1.0 - single, 0.0), limb);
    // Premultiplied: the blend is additive, so alpha leaves as zero and the colour carries it.
    return vec4<f32>(material.tint.rgb * alpha, 0.0);
}
