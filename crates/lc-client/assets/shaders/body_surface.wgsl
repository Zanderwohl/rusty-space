// A resolved body's surface.
//
// The pattern is a texture graph baked onto a cubemap -- see lc-client's surfaces module for
// which graph -- and the palette is the body's class's, from lc_world::surface. A body with a
// graph of its own has a color cubemap instead, and may have a cloud deck. Every cubemap is
// sampled on the body-fixed direction, which is what the mesh's local position already is, so
// the surface turns with the body and does not swim with the camera. The mesh's +Y is the pole.
// The cloud deck is lightcone/docs/07-rendering.md's "A cloud deck evolves", and the air over
// it that doc's "Air".

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}
#import lightcone::scatter::{air_of, crossing, scatter, star_depth, top_of}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) local_direction: vec3<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) center: vec3<f32>,
    @location(4) radius: f32,
    /// The spin axis, world.
    @location(5) pole: vec3<f32>,
}

struct BodySurfaceUniform {
    /// The two ends of the surface's color, linear.
    dark: vec4<f32>,
    light: vec4<f32>,
    /// World direction to the star. `w` is the ambient floor on the night side.
    to_star: vec4<f32>,
    /// `(color, contrast, clouds, unused)`: whether the color and cloud cubemaps are drawn.
    params: vec4<f32>,
    /// Starlight the surface reflects, as linear display light before the tone map.
    reflected: vec4<f32>,
    /// Light the body makes itself, in the same units. `w` is how far the pattern inverts in it.
    emitted: vec4<f32>,
    /// `(surface_reference, stops, 0, 0)`.
    exposure: vec4<f32>,
    /// Each weather slot's weight; `w` is the weather's mean.
    weather: vec4<f32>,
    /// Each slot's westward drift at the equator, radians.
    drift: vec4<f32>,
    /// `(cover, opacity, 0, 0)`: added to the drive, and the share of its alpha drawn.
    deck: vec4<f32>,
    /// Multiplies the deck's gray, linear.
    deck_tint: vec4<f32>,
    /// Starlight on a white surface facing the star, linear display light.
    starlight: vec4<f32>,
    /// scatter.wgsl's `Air`, packed: zero for a body without any.
    air_gas: vec4<f32>,
    air_haze: vec4<f32>,
    /// Each ground's albedo through the current mapping, then through the natural one: water,
    /// ice, growth, sand, rock, cloud.
    ground: array<vec4<f32>, 6>,
    ground_natural: array<vec4<f32>, 6>,
    /// Per band: display light from a blackbody at `thermal.x`, and its center in microns.
    bands: array<vec4<f32>, 7>,
    /// `(mean temperature K, how far the air evens day and night, 0, on)`.
    thermal: vec4<f32>,
    /// Per ground: `(B, V, R, I)`, then `(K, 10um, radio, inertia)`.
    emissivity: array<vec4<f32>, 12>,
}

/// Share of what the air scatters out of the beam that reaches the ground anyway.
const SKYLIGHT: f32 = 0.85;

/// Rec. 709, matching crate::tonemap.
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: BodySurfaceUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var pattern: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var pattern_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var color: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var weather_0: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var weather_1: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var weather_2: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var climate: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var mask_land: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var mask_ice: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var mask_growth: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(11) var mask_sand: texture_cube<f32>;

const CLOUD: i32 = 5;

// Where the deck's drive becomes cover, and the cover's color: earthlike-clouds.tgraph's
// "density ramp" and "cloud palette", which surfaces.rs's tests hold these to.
const DENSITY_FROM: f32 = 0.58;
const DENSITY_TO: f32 = 0.8;
const CLOUD_KNEE: f32 = 0.4;
const CLOUD_ALPHA: vec3<f32> = vec3<f32>(0.0, 0.45, 0.92);
const CLOUD_L: vec3<f32> = vec3<f32>(0.92, 0.91, 0.94);

/// `dir` turned eastward about the pole by `angle`.
fn turned(dir: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(c * dir.x + s * dir.z, dir.y, -s * dir.x + c * dir.z);
}

/// The deck at `dir`: linear gray, and straight alpha as cover.
fn deck(dir: vec3<f32>) -> vec4<f32> {
    // Eastward angular rate -cos(3 * latitude), in units of the equator's.
    let c = sqrt(max(1.0 - dir.y * dir.y, 0.0));
    let wind = 3.0 * c - 4.0 * c * c * c;
    // Minus the drift: a pattern moved east is found to the west.
    let mean = material.weather.w;
    let w = material.weather;
    let d = material.drift * wind;
    let weather = mean
        + w.x * (textureSample(weather_0, pattern_sampler, turned(dir, -d.x)).r - mean)
        + w.y * (textureSample(weather_1, pattern_sampler, turned(dir, -d.y)).r - mean)
        + w.z * (textureSample(weather_2, pattern_sampler, turned(dir, -d.z)).r - mean);
    let drive = weather + textureSample(climate, pattern_sampler, dir).r + material.deck.x;
    let density = saturate((drive - DENSITY_FROM) / (DENSITY_TO - DENSITY_FROM));
    let low = density < CLOUD_KNEE;
    let t = select((density - CLOUD_KNEE) / (1.0 - CLOUD_KNEE), density / CLOUD_KNEE, low);
    let alpha = select(mix(CLOUD_ALPHA.y, CLOUD_ALPHA.z, t), mix(CLOUD_ALPHA.x, CLOUD_ALPHA.y, t), low);
    let l = select(mix(CLOUD_L.y, CLOUD_L.z, t), mix(CLOUD_L.x, CLOUD_L.y, t), low);
    // A gray's Oklab lightness is the cube root of its linear value.
    return vec4<f32>(vec3<f32>(l * l * l) * material.deck_tint.rgb, saturate(alpha * material.deck.y));
}

/// How much of the texel at `dir` is water, ice, growth, sand and rock, summing to one.
fn grounds(dir: vec3<f32>) -> array<f32, 5> {
    let land = textureSample(mask_land, pattern_sampler, dir).r;
    let ice = textureSample(mask_ice, pattern_sampler, dir).r;
    let growth = textureSample(mask_growth, pattern_sampler, dir).r;
    let sand = textureSample(mask_sand, pattern_sampler, dir).r;
    let ground = land * (1.0 - ice);
    let dry = ground * (1.0 - growth);
    return array<f32, 5>((1.0 - land) * (1.0 - ice), ice, ground * growth, dry * sand, dry * (1.0 - sand));
}

/// Ground `k`'s emissivity in band `b`, or its inertia at `b = 7`.
fn emissivity_of(k: i32, b: i32) -> f32 {
    return material.emissivity[2 * k + b / 4][b % 4];
}

/// Second radiation constant, micron kelvins.
const C2_UM_K: f32 = 14388.0;

/// `1 - exp(-x)`, which loses everything to rounding at radio's tiny `x`.
fn one_minus_exp(x: f32) -> f32 {
    return select(1.0 - exp(-x), x * (1.0 - 0.5 * x), x < 0.01);
}

/// `B(um, t) / B(um, t0)`, written so neither exponent overflows: the blue end at a hundred
/// kelvin is `exp(300)`.
fn planck_ratio(um: f32, t: f32, t0: f32) -> f32 {
    let x = C2_UM_K / (um * max(t, 1.0));
    let x0 = C2_UM_K / (um * t0);
    return exp(clamp(x0 - x, -80.0, 80.0)) * one_minus_exp(x0) / max(one_minus_exp(x), 1.0e-20);
}

/// `v` turned by `angle` about unit `axis`, right-handed.
fn about(v: vec3<f32>, axis: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return v * c + cross(axis, v) * s + axis * dot(axis, v) * (1.0 - c);
}

/// A latitude's mean temperature over a day, from the body's mean: warmer at the equator.
fn zonal_k(sin_lat: f32) -> f32 {
    return material.thermal.x * (1.08 - 0.3 * sin_lat * sin_lat);
}

/// The ground's temperature at world normal `n`, with the damping `damp` its inertia and its
/// air give it. Undamped it is in balance with the sun overhead and cold at night; damped it is
/// its latitude's mean all day. The hottest hour comes after noon by as much as it is damped,
/// east of the point under the star, because the ground is still giving back the morning.
fn ground_k(n: vec3<f32>, sin_lat: f32, to_star: vec3<f32>, pole: vec3<f32>, damp: f32) -> f32 {
    let t0 = material.thermal.x;
    let mean = zonal_k(sin_lat);
    let sun = about(to_star, pole, 0.8 * damp);
    // A smooth max rather than a clamp: ground goes on giving back the day's heat after sunset,
    // for longer the more it holds. Clamped, a dry world's terminator was a ruled line.
    let c = dot(n, sun);
    let soft = 0.02 + 0.3 * damp;
    let day = 4.0 * pow(t0, 4.0) * 0.5 * (c + sqrt(c * c + soft * soft));
    let night = pow(0.5 * mean, 4.0);
    return pow(mix(max(day, night), pow(mean, 4.0), damp), 0.25);
}

/// What the ground and the cloud over it radiate, as display light. A cloud's emissivity is its
/// opacity, so it hides the ground at ten microns and not at 21 cm, and its tops are cold.
fn glow(dir: vec3<f32>, n: vec3<f32>, to_star: vec3<f32>, pole: vec3<f32>, cloud: f32) -> vec3<f32> {
    let w = grounds(dir);
    var inertia = 0.0;
    for (var k = 0; k < 5; k++) {
        inertia += w[k] * emissivity_of(k, 7);
    }
    let damp = 1.0 - (1.0 - inertia) * (1.0 - material.thermal.y);
    let t0 = material.thermal.x;
    let t_ground = ground_k(n, dir.y, to_star, pole, damp);
    let t_cloud = 0.8 * zonal_k(dir.y);
    var out = vec3<f32>(0.0);
    for (var b = 0; b < 7; b++) {
        var e = 0.0;
        for (var k = 0; k < 5; k++) {
            e += w[k] * emissivity_of(k, b);
        }
        let um = material.bands[b].w;
        let cover = cloud * emissivity_of(CLOUD, b);
        let radiated = (1.0 - cover) * e * planck_ratio(um, t_ground, t0) + cover * planck_ratio(um, t_cloud, t0);
        out += material.bands[b].rgb * radiated;
    }
    return out;
}

/// What the current band mapping sees of a color painted in the natural one, where the texel
/// is the grounds its masks say. rocky.tgraph's own mix: ice over everything, then land over
/// water, growth over dry ground, sand over rock.
///
/// The color is scaled by the ratio of the mix through the two mappings, so it keeps its detail
/// and in the natural mapping is exactly itself. A channel carrying I rather than red takes the
/// forest's red edge instead of its red.
fn banded(dir: vec3<f32>, own: vec3<f32>) -> vec3<f32> {
    let w = grounds(dir);
    var now = vec3<f32>(0.0);
    var natural = vec3<f32>(0.0);
    for (var k = 0; k < 5; k++) {
        now += w[k] * material.ground[k].rgb;
        natural += w[k] * material.ground_natural[k].rgb;
    }
    return own * now / max(natural, vec3<f32>(1.0e-4));
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_normal = normalize(mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index));
    // The mesh's own position is the body-fixed direction, because it is a unit sphere. So the
    // pattern turns with the body and is a function of nothing else.
    out.local_direction = normalize(vertex.position);
    out.world_position = world.xyz;
    out.center = world_from_local[3].xyz;
    out.radius = length(world_from_local[0].xyz);
    out.pole = normalize((world_from_local * vec4<f32>(0.0, 1.0, 0.0, 0.0)).xyz);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let contrast = material.params.y;
    let surface = textureSample(pattern, pattern_sampler, in.local_direction).r;
    let t = mix(0.5, surface, contrast);
    var albedo = mix(material.dark.rgb, material.light.rgb, t);
    var own = textureSample(color, pattern_sampler, in.local_direction).rgb;
    var cloud = deck(in.local_direction);
    if (material.params.w > 0.5) {
        own = banded(in.local_direction, own);
        cloud = vec4<f32>(cloud.rgb * material.ground[CLOUD].rgb / max(material.ground_natural[CLOUD].rgb, vec3<f32>(1.0e-4)), cloud.a);
    }
    albedo = mix(albedo, own, material.params.x);
    albedo = mix(albedo, cloud.rgb, cloud.a * material.params.z);

    // Lambert, with a soft terminator. A hard one is a straight line across the disc and reads
    // as a cut rather than as a horizon.
    let to_star = normalize(material.to_star.xyz);
    let lambert = dot(normalize(in.world_normal), to_star);
    let lit = smoothstep(-0.12, 0.25, lambert);
    var light = vec3<f32>(max(lit, material.to_star.w));

    // The air between the eye and the ground: it reddens the light reaching the ground near
    // the terminator, dims what leaves it, and adds what it scatters, which is the blue of a
    // day side and the brightening toward the limb.
    let air = air_of(material.air_gas, material.air_haze);
    var scattered = vec3<f32>(0.0);
    var through = vec3<f32>(1.0);
    if (air.height > 0.0) {
        let o = (view.world_position - in.center) / in.radius;
        let n = normalize(in.world_position - in.center);
        let ray = n - o;
        let t1 = length(ray);
        let d = ray / t1;
        let t0 = max(crossing(o, d, top_of(air)).x, 0.0);
        let s = scatter(o, d, t0, t1, to_star, air);
        // What the air takes from the beam it mostly scatters on down to the ground, which is
        // why clouds seen from space are white rather than the color of a sunset. Single
        // scattering alone would lose it.
        let beam = exp(-star_depth(n, to_star, air, false));
        light *= beam + (1.0 - beam) * SKYLIGHT;
        scattered = s.light * material.starlight.rgb;
        through = s.through;
    }

    // The body's own light, which does not care where the star is. In the optical it is zero
    // and this is the shader it always was; at ten microns it is the whole picture, and a
    // giant's night side stops being dark because nothing was lighting it in the first place.
    //
    // The pattern inverts: a belt is a gap in the cloud deck, so it reflects less and lets more
    // of the warm interior out. Mean-preserving about one, so the band the body is seen in
    // moves its pattern about rather than changing how much light it sends.
    let inversion = material.emitted.w;
    var emitted = material.emitted.rgb * mix(1.0 + inversion, 1.0 - inversion, t);
    if (material.thermal.w > 0.5) {
        emitted = glow(in.local_direction, normalize(in.world_normal), to_star, normalize(in.pole), cloud.a * material.params.z);
    }
    let linear = (material.reflected.rgb * albedo * light + emitted) * through + scattered;

    // The tone map of crate::tonemap, evaluated here rather than per body: the two terms mix
    // differently across the disc and the curve is logarithmic, so one level for the whole
    // surface gets the terminator wrong in any band where both terms matter.
    let reference = material.exposure.x;
    let stops = material.exposure.y;
    let luminance = dot(linear, LUMA);
    let peak = max(linear.r, max(linear.g, linear.b));
    var value = 0.0;
    if (luminance > 0.0 && reference > 0.0 && stops > 0.0) {
        value = clamp(log2(luminance / reference) / stops + 1.0, 0.0, 1.0);
    }
    let chroma = select(vec3<f32>(1.0), linear / peak, peak > 0.0);

    return vec4<f32>(chroma * value, 1.0);
}
