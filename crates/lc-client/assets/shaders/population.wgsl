// A population's envelope: swarms, belts and clouds, drawn as a volume.
//
// A population has no members to draw -- that is the entire point of 04-stellar-photometry.md,
// which replaces the roster with a distribution. So the renderer draws the distribution: a
// convex proxy whose fragments march the population's own density field and accumulate optical
// depth along the sightline. One shape covers every case, because the shape is not geometry any
// more -- it is two rows of a profile texture, and a belt and an isotropic swarm differ only in
// what those rows say.
//
// This is what makes a long way through the material look like a long way through the material.
// Painting a surface cannot: from inside a belt every sightline crosses the far wall exactly
// once and nearly face-on, so looking along the belt and looking at the pole came out the same.
//
// Rings take the surface path instead. A ring is a sheet, and a sheet has no inside.
//
// Per-vertex:
//   POSITION : the proxy in the population's own frame
//   NORMAL   : the surface normal there -- the pole, for a ring
//   DENSITY  : optical depth there, rings only
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
    @location(1) local_position: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) density: f32,
    // Constant over the instance, so flat rather than interpolated.
    @location(4) @interpolate(flat) camera_local: vec3<f32>,
}

struct PopulationUniform {
    tint: vec4<f32>,
    pole: vec4<f32>,
    opacity: f32,
    grain_frequency: f32,
    grain_strength: f32,
    seed: f32,
    limb_gain: f32,
    inside_fade: f32,
    inner: f32,
    slab: f32,
    volumetric: f32,
    band_to_display: array<vec4<f32>, BANDS>,
    band_material: array<vec4<f32>, BANDS>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: PopulationUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var profile: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var grain_volume: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var grain_sampler: sampler;

/// Bands carried from emission to display. Must match `em_spectra::BANDS`.
const BANDS: u32 = 7u;

/// Samples across a row of `profile`, and the rows. Must match `em_render::population_material`.
const PROFILE_SAMPLES: i32 = 128;
const PROFILE_LATITUDE: i32 = 0;
const PROFILE_RADIAL: i32 = 1;

/// Steps per sightline.
///
/// Fixed rather than derived from the span, because a step count that varies between
/// neighboring pixels quantises them differently and the seam is visible. The span is clipped
/// to the material before it is divided, so a thin belt gets the same thirty-two steps
/// concentrated in the thin part.
///
/// Measured on an M3 Pro at 1280x720, with two populations each covering the sky: **0.076 ms
/// per step per frame**, so thirty-two of them is about 2.4 ms. It is the whole of the cost —
/// everything else in this shader happens once. That scales with pixels and nothing else, which
/// is why the shells are marched at half resolution by `haze.rs` rather than with fewer steps
/// here: a Metal trace of the default view put the pass at 1.25 ms full size and 0.56 ms at
/// half, composite included.
///
/// Against a ninety-six-step render, thirty-two is within a mean of one level in 255 and
/// twenty-four within one and a half. Eight is within five and a half, which shows.
const STEPS: i32 = 32;

/// Column past which no band's transmittance can still change the pixel.
///
/// In column rather than in optical depth, because the depths are per band now. Generous: the
/// largest extinction coefficient any population carries is of order one over the reference
/// ray, so a column of six is already several e-foldings in the deepest band.
const OPAQUE: f32 = 6.0;

/// Grains in one repeat of `grain_volume`. Must match `em_render::population_material::GRAIN_TILE`.
const GRAIN_TILE: f32 = 16.0;

/// The baked grain at `p`, in grains, in `0..1`. The volume tiles, so any seed offset is as good
/// as any other.
///
/// Level zero, explicitly: the march samples inside a loop that can `break`, where an implicit
/// derivative is not allowed, and the volume has no mips to choose between anyway.
fn grain_at(p: vec3<f32>) -> f32 {
    let seeded = p + vec3<f32>(material.seed, material.seed * 2.7, material.seed * 1.3);
    return textureSampleLevel(grain_volume, grain_sampler, seeded / GRAIN_TILE, 0.0).r;
}

/// Granularity, in `-1..1` about zero.
fn grain(at: vec3<f32>) -> f32 {
    return 2.0 * grain_at(at * material.grain_frequency) - 1.0;
}

/// How far past the slab the grain can push the band's edge, so the march's own clip does not
/// cut off the material the warp adds.
fn grain_reach() -> f32 {
    return 1.0 + material.grain_strength;
}

/// One row of the profile, linearly interpolated. `u` is the row's own parameter, in `0..1`.
fn profile_at(u: f32, row: i32) -> f32 {
    let x = clamp(u, 0.0, 1.0) * f32(PROFILE_SAMPLES - 1);
    let i = i32(floor(x));
    let j = min(i + 1, PROFILE_SAMPLES - 1);
    let a = textureLoad(profile, vec2<i32>(i, row), 0).r;
    let b = textureLoad(profile, vec2<i32>(j, row), 0).r;
    return mix(a, b, x - f32(i));
}

/// The population's density at a point of its own frame, with the peak at one.
///
/// Separable in radius and latitude, which is not an approximation: the semi-major axis and
/// eccentricity distributions say where the material is radially and the inclination
/// distribution says how it is spread in latitude, and those are the three the population
/// carries. Nothing depends on longitude, which is what makes the whole thing two rows.
fn density_at(at: vec3<f32>) -> f32 {
    let radius = length(at);
    if (radius > 1.0 || radius < material.inner) {
        return 0.0;
    }
    let radial = profile_at((radius - material.inner) / max(1.0 - material.inner, 1e-6), PROFILE_RADIAL);
    // Latitude in units of the widest inclination, so a belt spends the whole row on the
    // twelve degrees it occupies rather than on the eighty-eight it does not.
    let sin_phi = abs(dot(at, material.pole.xyz)) / max(radius, 1e-6);

    // The grain moves the band's *edge* rather than dimming what is inside it.
    //
    // Scaling the density instead is what a surface shader did, and in a volume it disappears:
    // a sightline crosses several grains and averages them, so eighty-five per cent of contrast
    // per sample came out as eighteen per cent on screen and read as nothing. An edge does not
    // average — there is only one of it along any sightline — so that is where the texture goes,
    // and a band with a ragged edge reads as made of things where a soft gradient does not.
    //
    // The reference ray lies in the plane, where `sin_phi` is zero and the warp cannot reach it,
    // so the calibration is untouched by whatever this does.
    let warp = max(1.0 + material.grain_strength * grain(at), 0.05);
    let latitude = profile_at(sin_phi / max(material.slab * warp, 1e-6), PROFILE_LATITUDE);
    return radial * latitude;
}

/// Where a ray meets a sphere about the origin. `x > y` when it misses.
fn sphere_span(origin: vec3<f32>, direction: vec3<f32>, radius: f32) -> vec2<f32> {
    let b = dot(origin, direction);
    let c = dot(origin, origin) - radius * radius;
    let discriminant = b * b - c;
    if (discriminant <= 0.0) {
        return vec2<f32>(1.0, -1.0);
    }
    let root = sqrt(discriminant);
    return vec2<f32>(-b - root, -b + root);
}

/// Where a ray is inside the slab `|dot(p, pole)| <= half_height`.
///
/// A slab rather than the cone the inclinations really bound, because inside the unit sphere
/// the slab contains the cone and a slab cannot be got wrong. It is the whole of the saving on
/// a belt: without it a sightline crossing twelve degrees of material spends thirty of its
/// thirty-two steps in the empty sphere around it.
fn slab_span(origin: vec3<f32>, direction: vec3<f32>, half_height: f32) -> vec2<f32> {
    let along = dot(direction, material.pole.xyz);
    let above = dot(origin, material.pole.xyz);
    if (abs(along) < 1e-6) {
        if (abs(above) > half_height) {
            return vec2<f32>(1.0, -1.0);
        }
        return vec2<f32>(-3.0e38, 3.0e38);
    }
    let a = (-half_height - above) / along;
    let b = (half_height - above) / along;
    return vec2<f32>(min(a, b), max(a, b));
}

/// A per-pixel offset in `0..1`, so the steps of neighboring pixels do not line up and the
/// quantisation reads as film grain rather than as shells. Deliberately not a function of
/// time: two frames of a `--burst` have to be comparable.
fn dither(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

fn volume(in: VertexOutput) -> vec4<f32> {
    let origin = in.camera_local;
    let direction = normalize(in.local_position - origin);

    let sphere = sphere_span(origin, direction, 1.0);
    let slab = slab_span(origin, direction, material.slab * grain_reach());
    // Never behind the camera, and never behind the near plane either -- the shell is scaled
    // so one local unit is its outer radius, and the near plane is far below that.
    let enter = max(max(sphere.x, slab.x), 0.0);
    let leave = min(sphere.y, slab.y);
    let span = leave - enter;
    if (span <= 0.0) {
        discard;
    }

    // The column of material along the sightline, in the field's own units. One number, and
    // every band's optical depth is it times that band's own extinction coefficient -- so the
    // march does not care how many bands there are and costs the same for all seven.
    let step = span / f32(STEPS);
    let offset = dither(in.clip_position.xy);
    var column = 0.0;
    for (var i = 0; i < STEPS; i = i + 1) {
        let at = origin + direction * (enter + (f32(i) + offset) * step);
        column = column + density_at(at) * step;
        if (column > OPAQUE) {
            break;
        }
    }

    // What the instrument makes of it. A band contributes what the material radiates there
    // times how much of this sightline is filled *there*, and the two are different functions
    // of the band: at 21 cm a dust cloud has almost nothing in the way, so it contributes
    // almost nothing however brightly it would glow.
    var rgb = vec3<f32>(0.0);
    for (var b = 0u; b < BANDS; b = b + 1u) {
        let band = material.band_material[b];
        rgb = rgb + material.band_to_display[b].rgb * band.x * (1.0 - exp(-band.y * column));
    }
    // Premultiplied: the blend is additive, so alpha leaves as zero and the color carries it.
    return vec4<f32>(rgb, 0.0);
}

/// Rings. A sheet has no thickness to march, so its opacity is what one crossing covers, with
/// the path length through the sheet from the angle the sightline makes with it.
fn surface(in: VertexOutput) -> vec4<f32> {
    if (in.density <= 0.0) {
        discard;
    }
    let to_camera = normalize(view.world_position.xyz - in.world_position);
    let facing = abs(dot(in.normal, to_camera));
    let limb = 1.0 + material.limb_gain * (1.0 / max(facing, 0.08) - 1.0);

    let speckle = mix(1.0, grain_at(normalize(in.local_position) * material.grain_frequency),
        material.grain_strength);

    // `1 - (1 - t)^limb`, not `t * limb`. A sightline running along a sheet has unbounded path
    // length, and under a linear law it paints unbounded light: edge-on, Saturn's rings came
    // out at twice full white. At `limb` of one it is `t`, so the face-on case is untouched.
    let single = clamp(in.density * material.opacity * speckle * material.inside_fade, 0.0, 1.0);
    let alpha = 1.0 - pow(max(1.0 - single, 0.0), limb);
    return vec4<f32>(material.tint.rgb * alpha, 0.0);
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
    out.density = vertex.density;

    // The march wants the ray in the population's own frame, where the density field is two
    // rows of a texture. The camera is one point, so it converts once here rather than per
    // fragment, and the interpolator carries it flat.
    let local_from_world = mesh_functions::get_local_from_world(vertex.instance_index);
    out.camera_local = (local_from_world * vec4<f32>(view.world_position.xyz, 1.0)).xyz;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if (material.volumetric <= 0.5) {
        return surface(in);
    }
    // The proxy is a sphere about the origin drawn from both sides, so its outward normal at a
    // fragment is that fragment's own position. Keeping only the face the ray leaves through
    // gives exactly one fragment per covered pixel, inside the proxy or outside it, and
    // without depending on which way the mesh happens to be wound.
    if (dot(in.local_position - in.camera_local, in.local_position) <= 0.0) {
        discard;
    }
    return volume(in);
}
