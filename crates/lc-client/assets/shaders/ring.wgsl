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
// What the particles cover is the alpha, so a ring hides what is behind it in every band.
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
    // The direction to the star in the ring's own frame, where the planet is a sphere about the
    // origin. Constant over the instance.
    @location(3) @interpolate(flat) local_star: vec3<f32>,
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

/// How much starlight reaches `at` past the planet: one in the open, zero in its shadow.
///
/// `at` and `to_star` are in the ring's frame, where the planet is a sphere of radius `planet`
/// about the origin. `off` is how far the ray toward the star passes from the planet's center,
/// and `edge` the width the boundary is softened over, a pixel's worth.
fn sunlit(along: f32, off: f32, planet: f32, edge: f32) -> f32 {
    // Sunward of the planet's center nothing is in the way.
    if (along >= 0.0) {
        return 1.0;
    }
    return smoothstep(planet - 0.5 * edge, planet + 0.5 * edge, off);
}

/// `1 - exp(-x)`, by its first-order term where f32 would round it to nothing: `exp(-x)` is
/// exactly one under about 6e-8, which a generated ring reaches.
fn coverage_of(x: f32) -> f32 {
    return select(1.0 - exp(-x), x, x < 1.0e-4);
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
    let local_from_world = mesh_functions::get_local_from_world(vertex.instance_index);
    out.local_star = normalize((local_from_world * vec4<f32>(material.to_star.xyz, 0.0)).xyz);
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
    let along = dot(in.local_position, in.local_star);
    let off = sqrt(max(dot(in.local_position, in.local_position) - along * along, 0.0));
    // Outside the branch, because a derivative needs every neighbor to run it.
    let shadow = sunlit(along, off, material.params.z, max(fwidth(off), 1e-6));
    // Mean-preserving: the grain moves depth about, it does not add any.
    let speckle = 1.0 + material.grain.y * (2.0 * grain_at(radius, width) - 1.0);

    // Every band contributes its share of the pixel. Within a band the depth is one number, so
    // filtering the light band by band is the exact box filter of the light, not of the depth.
    var light = vec3<f32>(0.0);
    var least = 0.0;
    var covered = 0.0;
    var alpha = 0.0;
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
        let coverage = coverage_of(tau / mu);
        let sunward = lit_face(mu, mu0, tau);
        let face = select(unlit_face(mu, mu0, tau), sunward, sunlit_side);
        // The planet's shadow takes starlight from both faces; the particles' own glow stays.
        let here = reflected * face * shadow + emitted * coverage;
        // The floor follows the lighting: a thin ring is about as bright from either side, a thick
        // one is dark from the side away from the star, neither shines with no star on it, and
        // the planet's shadow darkens it as it does the light.
        let full = dot(reflected * sunward + emitted * coverage, LUMA);
        let lighting = select(0.0, dot(here, LUMA) / full, full > 0.0);
        light = light + share * here;
        least = least + share * material.params.y * opacity_of(coverage) * lighting;
        covered = covered + share;
        alpha = alpha + share * coverage;
    }
    if (covered <= 0.0) {
        discard;
    }

    let value = max(tone_value(light, material.exposure.x, material.exposure.y), least);
    // Premultiplied: the color is what the ring sends, and alpha the share of what is behind it
    // that the particles stop. Blended after the tone map, so the mix is of display values, as
    // the planet's own pixels are.
    return vec4<f32>(tone_chroma(light) * value, clamp(alpha, 0.0, 1.0));
}
