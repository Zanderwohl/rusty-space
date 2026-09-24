// A craft's field: em_render::field_material. FIELD_LAYER is 0 for the far wall, 1 for the inner
// rim and 2 for the near wall.
//
// Every wall's light is physical: a blackbody at the local temperature times the emissivity, in
// the same units as the starlight the lit surfaces reflect, through the same tone map. Nothing
// here sets how bright the field is, so at 400 K it is nothing in the visible and at 4 600 K it
// swamps everything.

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
    @location(1) world_normal: vec3<f32>,
    @location(2) local: vec3<f32>,
}

struct FieldUniform {
    state: vec4<f32>,
    mode: vec4<f32>,
    origin: vec4<f32>,
    to_star: vec4<f32>,
    starlight: vec4<f32>,
    exposure: vec4<f32>,
    hot_spots: array<vec4<f32>, HOT_SPOTS>,
    collapse: vec4<f32>,
    collapse_k: vec4<f32>,
    spectrum: array<vec4<f32>, RAMP>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: FieldUniform;

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);
// Mirrored by em_render::field_material::{HOT_SPOTS, RAMP}.
const HOT_SPOTS: i32 = 4;
const RAMP: i32 = 32;
// Mirrored by em_render::field_material::{RAMP_MIN_K, RAMP_MAX_K}.
const RAMP_MIN_K: f32 = 100.0;
const RAMP_MAX_K: f32 = 1.0e6;
const TAU: f32 = 6.2831853;

// The fill past which the field goes uneven; 30-the-field.md's 80%.
const UNEVEN_FILL: f32 = 0.8;

// A dielectric interface at normal incidence reflects about four percent.
const RIM_F0: f32 = 0.04;

// The film's thickness range, nanometers, and its index. A soap film's, which is what Clear is
// drawn as; thinner than this is black and thicker washes to white.
const FILM_THIN_NM: f32 = 250.0;
const FILM_THICK_NM: f32 = 700.0;
const FILM_INDEX: f32 = 1.33;
// A soap film's reflectance face-on, and how far its bands are allowed from white. Fully
// saturated, the bands read as paint rather than as a sheen.
const FILM_FACE_ON: f32 = 0.06;
const FILM_SATURATION: f32 = 0.4;

fn hash3(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yxz + 33.33);
    return fract((r.x + r.y) * r.z);
}

fn value_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let n000 = hash3(i);
    let n100 = hash3(i + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = hash3(i + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = hash3(i + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = hash3(i + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = hash3(i + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = hash3(i + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = hash3(i + vec3<f32>(1.0, 1.0, 1.0));
    return mix(
        mix(mix(n000, n100, u.x), mix(n010, n110, u.x), u.y),
        mix(mix(n001, n101, u.x), mix(n011, n111, u.x), u.y),
        u.z,
    );
}

fn fbm(p: vec3<f32>) -> f32 {
    return 0.55 * value_noise(p) + 0.3 * value_noise(p * 2.03 + 17.0) + 0.15 * value_noise(p * 4.1 + 41.0);
}

/// A blackbody at `kelvin` through the host's bands, linear display light.
///
/// Interpolated in `1/T` between ramp entries rather than in `T`: on the Wien side the log of
/// the radiance is linear in `1/T`, and that side is where a field lives. Past the top it is
/// Rayleigh-Jeans, linear in `T`.
fn blackbody(kelvin: f32) -> vec3<f32> {
    if (kelvin <= RAMP_MIN_K) {
        return vec3<f32>(0.0);
    }
    let span = log(RAMP_MAX_K / RAMP_MIN_K);
    let at = clamp(log(kelvin / RAMP_MIN_K) / span * f32(RAMP - 1), 0.0, f32(RAMP - 1));
    let i = min(i32(floor(at)), RAMP - 2);
    let k0 = RAMP_MIN_K * exp(span * f32(i) / f32(RAMP - 1));
    let k1 = RAMP_MIN_K * exp(span * f32(i + 1) / f32(RAMP - 1));
    let t = clamp((1.0 / k0 - 1.0 / min(kelvin, RAMP_MAX_K)) / (1.0 / k0 - 1.0 / k1), 0.0, 1.0);
    let log_rgb = mix(material.spectrum[i].rgb, material.spectrum[i + 1].rgb, t);
    return exp2(log_rgb) * max(kelvin / RAMP_MAX_K, 1.0);
}

/// Where the collapse has got to: `(debris radius as a multiple of the envelope, flash, cooled)`,
/// or `x` zero while the field stands. `flash` falls from one; `cooled` runs from zero to one
/// over the afterglow.
fn collapse_phase() -> vec3<f32> {
    let since = material.collapse.x;
    if (since < 0.0) {
        return vec3<f32>(0.0);
    }
    let flash_s = max(material.collapse.y, 1e-3);
    let cooled = clamp(since / max(material.collapse.z, 1e-3), 0.0, 1.0);
    // Fast at first and coasting: debris is thrown, then only drifts.
    let grow = 1.0 + (material.collapse.w - 1.0) * (1.0 - pow(1.0 - cooled, 3.0));
    // Over in `flash_s` and not merely fading: a spike of ten million kelvin is so far over any
    // exposure that an exponential tail stays white for tens of flash lengths.
    let flash = pow(max(1.0 - since / flash_s, 0.0), 2.0);
    return vec3<f32>(grow, flash, cooled);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    var local = vertex.position;
    var normal = vertex.normal;
    let phase = collapse_phase();
    if (phase.x > 0.0) {
        // The envelope rounds into a shell in the first moments, then carries on outward.
        let rounding = smoothstep(0.0, 0.05, phase.z);
        let radius = length(local);
        let dir = local / max(radius, 1e-6);
        let sphere = material.collapse_k.z * phase.x;
        local = dir * mix(radius * phase.x, sphere, rounding);
        normal = normalize(mix(normal, dir, rounding));
    }
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(local, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_position = world.xyz;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(normal, vertex.instance_index);
    out.local = local;
    return out;
}

/// The tone map of lc-client's `tonemap.rs`, with the overflow carried per channel past the
/// window as `plume.wgsl` carries it, so a field too bright for the exposure blooms and keeps
/// its structure instead of becoming one flat shape.
fn exposed(linear: vec3<f32>) -> vec3<f32> {
    let reference = material.exposure.x;
    let stops = material.exposure.y;
    let luminance = dot(linear, LUMA);
    let peak = max(linear.r, max(linear.g, linear.b));
    if (!(luminance > 0.0) || reference <= 0.0 || stops <= 0.0) {
        return vec3<f32>(0.0);
    }
    let chroma = linear / peak;
    let level = clamp(log2(luminance / reference) / stops + 1.0, 0.0, 1.0);
    let over = clamp(
        log2(max(linear, vec3<f32>(1e-30)) / reference),
        vec3<f32>(0.0),
        vec3<f32>(12.0),
    );
    return chroma * level + over * material.exposure.z;
}

/// How black the surface is here, `[0, 1]`: the new mode inside the sweep, the old outside.
fn blackness(local: vec3<f32>) -> f32 {
    let progress = clamp(material.mode.z, 0.0, 1.0);
    if (progress >= 1.0) {
        return material.mode.x;
    }
    let reach = max(material.mode.w, 1e-6);
    let front = progress * reach * 1.05;
    let edge = 0.04 * reach;
    let d = distance(local, material.origin.xyz);
    let inside = 1.0 - smoothstep(front - edge, front, d);
    return mix(material.mode.y, material.mode.x, inside);
}

/// The factor the local emissive power is over the field's mean: hot spots and the flicker.
fn unevenness(world_normal: vec3<f32>, local: vec3<f32>) -> f32 {
    let fill = clamp(material.state.y, 0.0, 1.0);
    var power = 1.0;
    // A beam heats the side facing it, and the heat spreads across the envelope as it fills.
    let width = mix(0.25, 1.1, fill);
    for (var i = 0; i < HOT_SPOTS; i = i + 1) {
        let spot = material.hot_spots[i];
        if (spot.w > 0.0) {
            let angle = acos(clamp(dot(world_normal, normalize(spot.xyz)), -1.0, 1.0));
            let a = angle / width;
            power = power + spot.w * exp(-a * a);
        }
    }

    // Past the limit's approach the glow goes uneven, and faster as it nears the limit. Two
    // rates beating against each other so it stutters rather than pulsing.
    let stress = smoothstep(UNEVEN_FILL, 1.0, fill);
    if (stress > 0.0) {
        let t = material.state.w;
        let rate = mix(2.0, 14.0, stress * stress);
        let dir = normalize(local);
        let mottle = fbm(dir * 3.0 + vec3<f32>(0.0, 0.0, t * rate));
        let stutter = value_noise(vec3<f32>(t * rate * 1.7, 5.3, 1.1));
        let swing = (mottle - 0.5) * 1.8 + (stutter - 0.5) * 1.0;
        // Floored well above zero: a dip to black reads as a hole burnt in the envelope.
        power = power * max(1.0 + stress * swing, 0.6);
    }
    return power;
}

/// A soap film's reflectance at the display's three primaries, mean one, drifting with the clock.
fn film(local: vec3<f32>, mu: f32) -> vec3<f32> {
    let t = material.state.w;
    let dir = normalize(local);
    let drift = fbm(dir * 1.6 + vec3<f32>(t * 0.05, t * 0.03, -t * 0.04));
    let thickness = mix(FILM_THIN_NM, FILM_THICK_NM, drift);
    // Refracted angle inside the film, then the round-trip phase at 610, 550 and 465 nm.
    let sin_i2 = 1.0 - mu * mu;
    let cos_t = sqrt(max(1.0 - sin_i2 / (FILM_INDEX * FILM_INDEX), 0.0));
    let path = 2.0 * FILM_INDEX * thickness * cos_t;
    let lambda = vec3<f32>(610.0, 550.0, 465.0);
    let s = sin(TAU * path / lambda);
    return 2.0 * s * s;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) facing: bool) -> @location(0) vec4<f32> {
    let to_eye = normalize(view.world_position - in.world_position);
    let outward = normalize(in.world_normal);
    let n = select(-outward, outward, facing);
    // Floored so the path through the shell at the silhouette is long but not infinite, and a
    // Clear edge stays something the ship shows through.
    let mu = clamp(dot(n, to_eye), 0.15, 1.0);
    let phase = collapse_phase();
    let collapsing = phase.x > 0.0;

#if FIELD_LAYER == 1
    // The inner layer: the interface catching starlight at a glance, and nothing more. It goes
    // with the envelope.
    if (collapsing) {
        return vec4<f32>(0.0);
    }
    let fresnel = RIM_F0 + (1.0 - RIM_F0) * pow(1.0 - mu, 5.0);
    let lit = abs(dot(outward, normalize(material.to_star.xyz)));
    let rim = material.starlight.rgb * fresnel * (0.3 + 0.7 * lit);
    return vec4<f32>(exposed(rim), 0.0);
#else
    let kelvin = material.state.x;
    var linear = vec3<f32>(0.0);
    var opacity = 0.0;

    if (collapsing) {
        // Debris: the flash over the whole shell, then clumps that thin as the sphere grows and
        // cool from the field's limit.
        let dir = normalize(in.local);
        let clumps = fbm(dir * 7.0 + 3.0);
        let thin = mix(0.35, 0.6, phase.z);
        let cover = smoothstep(thin, thin + 0.12, clumps) * sqrt(1.0 - phase.z);
        let cooling = material.collapse_k.y * pow(1.0 - phase.z, 0.6) + 300.0;
        let spike = blackbody(material.collapse_k.x) * phase.y;
        linear = spike + blackbody(cooling) * cover;
        opacity = max(cover, phase.y);
    } else {
        let black = blackness(in.local);
        let absorbs = mix(clamp(material.state.z, 0.0, 1.0), 1.0, black);
        // Kirchhoff: emissivity is absorptivity, and a thin shell's grows toward one along a
        // grazing path. The same number is how much of what is behind the wall it takes out.
        let emissivity = 1.0 - pow(1.0 - absorbs, 1.0 / mu);
        let power = unevenness(outward, in.local);
        linear = blackbody(kelvin * pow(power, 0.25)) * emissivity;

        // Clear reflects what it does not absorb, as a film does: almost nothing face-on and
        // all of it at a glance, so the ship shows through the middle. How much it reflects in
        // total is the photometry's business; this is only where on the disc it goes.
        let lit = abs(dot(n, normalize(material.to_star.xyz)));
        let glance = FILM_FACE_ON + (1.0 - FILM_FACE_ON) * pow(1.0 - mu, 3.0);
        let reflects = (1.0 - absorbs) * glance * lit;
        let banded = mix(vec3<f32>(1.0), film(in.local, mu), FILM_SATURATION);
        linear = linear + material.starlight.rgb * banded * reflects;
        opacity = emissivity;
    }

#if FIELD_LAYER == 0
    return vec4<f32>(exposed(linear), 0.0);
#else
    return vec4<f32>(exposed(linear), clamp(opacity, 0.0, 1.0));
#endif
#endif
}
