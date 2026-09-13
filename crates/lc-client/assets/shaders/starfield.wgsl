// The observer's sky: one camera-facing quad per star, shaded from physics.
//
// Descended from assets/shaders/starfield.wgsl in the Exotic Matters app, which established
// the technique: bake four vertices per star into one mesh, expand the quad in view space in
// the vertex stage, and clamp to background depth. What is different here is that nothing
// about a star's *appearance* is baked. The catalogue shader bakes a colour and a brightness
// because its only input is an apparent magnitude; this one bakes a temperature and a radius
// and derives the rest per frame, because aberration, Doppler shift, the band mapping and the
// exposure all change while the ship flies and re-uploading the mesh for them does not scale.
//
// Per-vertex, baked:
//   POSITION : star position relative to the bake origin, light-years, render axes
//   CORNER   : quad corner in [-1, 1]^2, also the radial falloff coordinate
//   PARAMS   : (effective temperature K, radius m, unused, unused)
//
// Everything else is a uniform. See lightcone/docs/07-rendering.md.

#import bevy_pbr::mesh_view_bindings::view

const PI: f32 = 3.14159265;
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
const BANDS: u32 = 7u;

struct StarfieldUniform {
    // Rows of the band-to-display matrix, one entry per band: (r, g, b, unused).
    band_to_display: array<vec4<f32>, 7>,
    // Ship velocity as a fraction of c, render axes.
    beta: vec4<f32>,
    // Where the ship is relative to the bake origin, light-years.
    ship_offset_ly: vec4<f32>,
    reference: f32,
    point_stops: f32,
    min_radius_rad: f32,
    max_radius_rad: f32,
    glow_radius_gain: f32,
    brightness: f32,
    // Lookup domain: index = (log2(T) - log_t_min) * log_t_scale.
    log_t_min: f32,
    log_t_scale: f32,
    lut_samples: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: StarfieldUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var band_lut: texture_2d<f32>;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) corner: vec2<f32>,
    @location(2) params: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) colour: vec3<f32>,
};

fn lorentz(beta: vec3<f32>) -> f32 {
    return inverseSqrt(max(1.0 - dot(beta, beta), 1e-12));
}

/// Where a source actually along `to_source` appears to be, for an observer at `beta`.
///
/// The sky compresses forward: at beta = 0.5 a source 90 degrees off the bow appears at 60.
fn aberrate(to_source: vec3<f32>, beta: vec3<f32>) -> vec3<f32> {
    let n = -to_source;
    let g = lorentz(beta);
    let ndb = dot(n, beta);
    let num = n + beta * (g * (g / (g + 1.0) * ndb - 1.0));
    return -normalize(num / (g * (1.0 - ndb)));
}

/// Observed over emitted frequency. Above 1 is a blueshift.
fn doppler(to_source: vec3<f32>, beta: vec3<f32>) -> f32 {
    return lorentz(beta) * (1.0 + dot(beta, to_source));
}

/// Band radiance of a blackbody at `teff`, from the table em-spectra generated.
///
/// The table holds log2 of the radiance, and the interpolation is done there rather than in
/// the radiance itself: across the Wien tail the value moves by decades between samples and
/// a linear blend of the raw numbers would be wrong by most of the interval.
fn band_radiance(band: u32, teff: f32) -> f32 {
    let last = material.lut_samples - 1.0;
    let at = clamp((log2(teff) - material.log_t_min) * material.log_t_scale, 0.0, last);
    let i = i32(floor(at));
    let j = i32(min(f32(i) + 1.0, last));
    let frac = at - floor(at);
    let a = textureLoad(band_lut, vec2<i32>(i, i32(band)), 0).r;
    let b = textureLoad(band_lut, vec2<i32>(j, i32(band)), 0).r;
    return exp2(mix(a, b, frac));
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.corner = vertex.corner;

    // Both terms are small: the bake origin follows the ship, so this subtraction never
    // cancels the way differencing two absolute interstellar positions in f32 would.
    let rel = vertex.position - material.ship_offset_ly.xyz;
    let distance_ly = max(length(rel), 1e-9);
    let to_source = rel / distance_ly;

    let beta = material.beta.xyz;
    // A blackbody seen with Doppler factor D is exactly a blackbody at D times the
    // temperature -- B_nu/nu^3 is invariant and Planck's law depends only on nu/T -- so the
    // beaming needs no separate D^4 term. It is already in the band integral.
    let teff = max(vertex.params.x * doppler(to_source, beta), 1.0);

    // Ratio before squaring: distance in metres squared overflows f32 past a couple of
    // thousand light-years, and the ratio never does.
    let shrink = vertex.params.y / (distance_ly * 9.4607305e15);
    let geometry = PI * shrink * shrink;

    var linear = vec3<f32>(0.0);
    for (var b = 0u; b < BANDS; b = b + 1u) {
        linear = linear + material.band_to_display[b].rgb * band_radiance(b, teff) * geometry;
    }

    // The tone map of crate::tonemap, evaluated per star: stops above the window's top, with
    // chroma kept separate so brightness can drive size instead of only value.
    let luminance = dot(linear, LUMA);
    let peak = max(linear.r, max(linear.g, linear.b));
    var above = -1e9;
    if (luminance > 0.0 && material.reference > 0.0) {
        above = log2(luminance / material.reference);
    }
    let chroma = select(vec3<f32>(1.0), linear / peak, peak > 0.0);
    let level = clamp(1.0 + above / max(material.point_stops, 1e-6), 0.0, 1.0);
    let glow = clamp(above, 0.0, 12.0);

    out.colour = chroma * level * material.brightness;

    // Brightness past the top of the window widens the halo rather than whitening the core.
    let radius_rad = mix(material.min_radius_rad, material.max_radius_rad, level)
        + material.glow_radius_gain * glow * material.min_radius_rad;

    // w = 0 drops the camera's translation, so the sky depends only on where it is pointed.
    // The quad centre sits one unit down the view ray, which makes the corner offset equal to
    // the angle subtended, so radius_rad is an angular radius with no projection arithmetic.
    let seen = aberrate(to_source, beta);
    let dir_view = normalize((view.view_from_world * vec4<f32>(seen, 0.0)).xyz);
    let pos_view = dir_view + vec3<f32>(vertex.corner, 0.0) * radius_rad;
    var clip = view.clip_from_view * vec4<f32>(pos_view, 1.0);
    clip.z = clip.w;
    // A star behind the camera must not wrap to the front.
    if (dir_view.z > 0.0) {
        clip = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    out.clip_position = clip;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let r = length(in.corner);
    if (r > 1.0) {
        discard;
    }
    return vec4<f32>(in.colour * (1.0 - r), 1.0);
}
