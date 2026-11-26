// A planet's rings: a flat annulus, with the bands evaluated per fragment.
//
// The mesh only has to cover the annulus. Where the material is, and how deep, comes from the
// band list in the uniform, box-filtered over the pixel's radial footprint: an edge is sharp at
// any zoom, and a ringlet or gap narrower than a pixel dims or brightens that pixel by its share
// of it rather than appearing or vanishing with the sample position.
//
// Each band is lit as a sheet of particles by single scattering, through the current band
// mapping as a planet is, and tone-mapped with the planet's own curve. Over that sits a display
// floor, so a thin ring stays faintly visible where its true brightness is under the window.
//
// Per-vertex:
//   POSITION : the annulus in its own frame, unit outer radius, centered on the planet
//   NORMAL   : the ring's pole

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}
#import "shaders/tonemap.wgsl"::{LUMA, tone_chroma, tone_value}

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

/// The shallowest a sightline is taken to cross the sheet. Edge-on the path is unbounded, and a
/// real ring is meters thick, so past this the sheet is opaque anyway.
const MIN_MU: f32 = 0.01;

/// Below this coverage the floor is nothing, as `envelope::FAINTEST` is for a population.
const FAINTEST: f32 = 1.0e-13;

/// Within this of each other the unlit face's formula is taken at its limit.
const SAME_MU: f32 = 1.0e-3;

struct RingUniform {
    bands: array<vec4<f32>, RING_BANDS>,
    params: vec4<f32>,
    reflected_rocky: vec4<f32>,
    reflected_icy: vec4<f32>,
    emitted_rocky: vec4<f32>,
    emitted_icy: vec4<f32>,
    to_star: vec4<f32>,
    exposure: vec4<f32>,
    grain: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: RingUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var grain_volume: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var grain_sampler: sampler;

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

/// The sunlit face of a sheet at optical depth `tau`, seen at `mu` and lit at `mu0` (cosines
/// from the pole), as a share of what a fully filled sheet sends back face-on.
///
/// Single scattering, `2 mu0 / (mu + mu0) (1 - exp(-tau (1/mu + 1/mu0)))`: scaled so a thick
/// sheet lit and seen face-on is a Lambert surface of the particles' albedo, which is how the
/// planet beside it is shaded.
fn lit_face(mu: f32, mu0: f32, tau: f32) -> f32 {
    if (mu0 <= 0.0) {
        return 0.0;
    }
    return 2.0 * mu0 / (mu + mu0) * (1.0 - exp(-tau * (1.0 / mu + 1.0 / mu0)));
}

/// The face away from the star: light scattered once on its way through.
/// `2 mu0 / (mu - mu0) (exp(-tau/mu) - exp(-tau/mu0))`, at its limit where `mu` is `mu0`.
fn unlit_face(mu: f32, mu0: f32, tau: f32) -> f32 {
    if (mu0 <= 0.0) {
        return 0.0;
    }
    if (abs(mu - mu0) < SAME_MU) {
        return 2.0 * mu0 * tau / (mu * mu) * exp(-tau / mu);
    }
    return 2.0 * mu0 / (mu - mu0) * (exp(-tau / mu) - exp(-tau / mu0));
}

/// `envelope::opacity_of`: a coverage as a display level, by its fourth root.
fn opacity_of(coverage: f32) -> f32 {
    if (coverage <= FAINTEST) {
        return 0.0;
    }
    return clamp(pow(coverage, 0.25), 0.0, 1.0);
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
    let half_width = 0.5 * width;

    let pole = normalize(in.normal);
    let to_camera = normalize(view.world_position.xyz - in.world_position);
    let seen = dot(pole, to_camera);
    let lit = dot(pole, material.to_star.xyz);
    let mu = max(abs(seen), MIN_MU);
    let mu0 = abs(lit);
    let sunlit_side = seen * lit > 0.0;
    // Mean-preserving: the grain moves depth about, it does not add any.
    let speckle = 1.0 + material.grain.y * (2.0 * grain_at(radius, width) - 1.0);

    // Every band contributes its share of the pixel. Within a band the depth is one number, so
    // filtering the light band by band is the exact box filter of the light, not of the depth.
    var light = vec3<f32>(0.0);
    var least = 0.0;
    var covered = 0.0;
    let count = min(u32(material.params.x), RING_BANDS);
    for (var i = 0u; i < count; i = i + 1u) {
        let band = material.bands[i];
        // Edges relative to `radius` before clamping: near the outer edge a close pixel is a
        // millionth of the radius wide, and offsetting the radius first loses most of its bits.
        let share = (clamp(band.y - radius, -half_width, half_width) - clamp(band.x - radius, -half_width, half_width)) / width;
        if (share <= 0.0) {
            continue;
        }
        let tau = band.z * speckle;
        let reflected = mix(material.reflected_rocky.rgb, material.reflected_icy.rgb, band.w);
        let emitted = mix(material.emitted_rocky.rgb, material.emitted_icy.rgb, band.w);
        let coverage = 1.0 - exp(-tau / mu);
        let sunward = lit_face(mu, mu0, tau);
        let face = select(unlit_face(mu, mu0, tau), sunward, sunlit_side);
        let here = reflected * face + emitted * coverage;
        // The floor follows the lighting: a thin ring is about as bright from either side, a thick
        // one is dark from the side away from the star, and neither shines with no star on it.
        let full = dot(reflected * sunward + emitted * coverage, LUMA);
        let lighting = select(0.0, dot(here, LUMA) / full, full > 0.0);
        light = light + share * here;
        least = least + share * material.params.y * opacity_of(coverage) * lighting;
        covered = covered + share;
    }
    if (covered <= 0.0) {
        discard;
    }

    let value = max(tone_value(light, material.exposure.x, material.exposure.y), least);
    // Premultiplied and additive: alpha leaves as zero and the color carries it.
    return vec4<f32>(tone_chroma(light) * value, 0.0);
}
