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
//   PARAMS   : (effective temperature K, radius m, corona seed, unused)
//   WARM     : (population temperature K, its radiance over the star's disc, grey deficit, -)
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
    overflow_gain: f32,
    halo_gain: f32,
    /// How much angular structure the glare carries. Zero leaves it a smooth halo.
    corona_strength: f32,
    /// Filaments per radian of sky. Higher is finer structure.
    corona_frequency: f32,
    /// Exponent of the glare's power-law falloff from the source.
    halo_falloff: f32,
    /// Shortest streamer, and how much longer the longest is, as fractions of the quad.
    corona_reach_min: f32,
    corona_reach_span: f32,
    /// Width of the fade at a streamer's tip.
    corona_fade: f32,
    /// Brightness between the streamers, and how much they add on top.
    corona_floor: f32,
    corona_gain: f32,
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
    @location(3) warm: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) colour: vec3<f32>,
    /// Where the source itself ends and the glare begins, as a fraction of the quad.
    @location(2) core: f32,
    /// Offset from the star in the plane of the sky, world axes. Its *direction* is the angle
    /// around the star; its length is the distance out.
    @location(3) sky: vec3<f32>,
    /// World direction to the star, the axis the corona is arranged about.
    @location(4) axis: vec3<f32>,
    @location(5) seed: f32,
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

// --- corona ----------------------------------------------------------------------------
//
// Not physical, and deliberately so. A real corona is a millionth of the photosphere and
// invisible without occulting it. This ship occults it: it has no eyes, only a pipeline, and
// 07-rendering.md already hands the player the band matrix on the same grounds.
//
// The structure is ridged fractal noise sampled on the *normalised* direction from the star.
// Normalising discards the radial coordinate, so the pattern is constant along every ray out
// of the star and the filaments come out radial without being asked for. It is a function of
// world direction and a per-star seed and of nothing else, so it does not swim when the camera
// turns, it is the same for every client, and flying around a star shows its other side.

fn hash31(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn value_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    // Smoothstep weights, so the lattice does not show as a grid.
    let w = f * f * (3.0 - 2.0 * f);
    let c000 = hash31(i + vec3<f32>(0.0, 0.0, 0.0));
    let c100 = hash31(i + vec3<f32>(1.0, 0.0, 0.0));
    let c010 = hash31(i + vec3<f32>(0.0, 1.0, 0.0));
    let c110 = hash31(i + vec3<f32>(1.0, 1.0, 0.0));
    let c001 = hash31(i + vec3<f32>(0.0, 0.0, 1.0));
    let c101 = hash31(i + vec3<f32>(1.0, 0.0, 1.0));
    let c011 = hash31(i + vec3<f32>(0.0, 1.0, 1.0));
    let c111 = hash31(i + vec3<f32>(1.0, 1.0, 1.0));
    let x00 = mix(c000, c100, w.x);
    let x10 = mix(c010, c110, w.x);
    let x01 = mix(c001, c101, w.x);
    let x11 = mix(c011, c111, w.x);
    return mix(mix(x00, x10, w.y), mix(x01, x11, w.y), w.z);
}

/// Ridged fractal noise: `1 - |2n - 1|` turns the smooth field's zero crossings into creases,
/// which is what makes threads rather than blobs.
fn filaments(direction: vec3<f32>, seed: f32) -> f32 {
    let p = direction * material.corona_frequency + vec3<f32>(seed, seed * 1.7, seed * 2.3);
    var sum = 0.0;
    var amplitude = 0.66;
    var frequency = 1.0;
    // Three octaves, not more. The fine ones read as fur, and a corona is a few broad
    // streamers. Squared rather than cubed for the same reason: each extra power narrows the
    // crease, and these are meant to be thick.
    for (var i = 0u; i < 3u; i = i + 1u) {
        let n = value_noise(p * frequency);
        let ridge = 1.0 - abs(2.0 * n - 1.0);
        sum = sum + amplitude * ridge * ridge;
        frequency = frequency * 2.4;
        amplitude = amplitude * 0.42;
    }
    return clamp(sum, 0.0, 1.5);
}

/// How far one streamer reaches, `[0, 1]`.
///
/// A second field on the *same* angular scale as the threads but a different seed, so length
/// and brightness are not the same number.
///
/// The scale matters as much as the decorrelation. Driving both from one field made every long
/// streamer also the brightest, which the eye picks up at once; sampling the length at half the
/// frequency replaced the streamers with half a dozen broad lobes, because the thing being
/// varied was no longer a streamer.
fn reach_of(direction: vec3<f32>, seed: f32) -> f32 {
    let p = direction * material.corona_frequency
        + vec3<f32>(seed * 3.1 + 41.0, seed * 0.7 + 17.0, seed * 1.3 + 29.0);
    return value_noise(p);
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
    // Where the ship sees it, which is not where it is.
    let seen = aberrate(to_source, beta);
    // A blackbody seen with Doppler factor D is exactly a blackbody at D times the
    // temperature -- B_nu/nu^3 is invariant and Planck's law depends only on nu/T -- so the
    // beaming needs no separate D^4 term. It is already in the band integral.
    let teff = max(vertex.params.x * doppler(to_source, beta), 1.0);

    // The star's true angular radius, and its solid angle. Ratio before squaring: distance in
    // metres squared overflows f32 past a couple of thousand light-years, and the ratio never
    // does.
    let disc_rad = vertex.params.y / (distance_ly * 9.4607305e15);
    let geometry = PI * disc_rad * disc_rad;

    // A population absorbs starlight and re-emits it as a blackbody at the temperature its
    // orbit sets. Occultation is grey and removes; re-emission is cold and adds, which in the
    // thermal infrared can be a hundred times what the star itself puts out. Both shift with
    // the same Doppler factor, because both are blackbodies.
    let warm_t = max(vertex.warm.x * doppler(to_source, beta), 1.0);
    let warm_scale = vertex.warm.y;
    let survives = 1.0 - vertex.warm.z;

    var linear = vec3<f32>(0.0);
    for (var b = 0u; b < BANDS; b = b + 1u) {
        var radiance = band_radiance(b, teff) * survives;
        if (warm_scale > 0.0) {
            radiance = radiance + band_radiance(b, warm_t) * warm_scale;
        }
        linear = linear + material.band_to_display[b].rgb * radiance * geometry;
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
    let glow = clamp(above, 0.0, 24.0);

    // Overflow leaves as an HDR value rather than clipping to white. A star seen from sixty
    // astronomical units is twenty-four stops over the window; without this it renders exactly
    // like one that is barely over, and arriving somewhere looks like arriving nowhere.
    out.colour = chroma * material.brightness * (level + glow * material.overflow_gain);

    // The source itself: its disc if that is resolvable, otherwise the smallest thing worth
    // drawing. At sixty astronomical units a sun is a fiftieth of a pixel across, and it is
    // bright rather than big.
    let core_rad = max(disc_rad, material.min_radius_rad);
    // The glare around it, which is what grows with brightness.
    let glare_rad = mix(material.min_radius_rad, material.max_radius_rad, level)
        + material.glow_radius_gain * glow * material.min_radius_rad;

    // The quad covers the glare; the core is a fraction of it. One quad drawing a single
    // filled disc made a star a hundred and sixty pixels across into a flat white ball, with
    // the disc it actually has swamped inside it.
    let radius_rad = max(core_rad, glare_rad);
    out.core = clamp(core_rad / radius_rad, 0.0, 1.0);

    // Offset from the star in the plane of the sky, in world axes so the pattern is anchored
    // to the star rather than to the camera. Normalising this in the fragment discards the
    // distance out and leaves only the angle around the star, which is what makes every
    // feature a radial thread instead of a blob.
    let right_world = (view.world_from_view * vec4<f32>(1.0, 0.0, 0.0, 0.0)).xyz;
    let up_world = (view.world_from_view * vec4<f32>(0.0, 1.0, 0.0, 0.0)).xyz;
    out.sky = right_world * vertex.corner.x + up_world * vertex.corner.y;
    out.axis = seen;
    out.seed = vertex.params.z;

    // w = 0 drops the camera's translation, so the sky depends only on where it is pointed.
    // The quad centre sits one unit down the view ray, which makes the corner offset equal to
    // the angle subtended, so radius_rad is an angular radius with no projection arithmetic.
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
    // A bright edge to the source, and a soft fall outside it. For a distant star the core
    // fills the quad and this reduces to the linear falloff it had before.
    let core = 1.0 - smoothstep(in.core * 0.8, in.core, r);

    // A power law out from the source, not a fade in from the edge of the quad. A corona falls
    // off with distance from the star; `pow(1 - r, n)` is a property of where the quad happens
    // to end, which is a different shape and reads as a ball.
    let rr = max(r, in.core);
    let profile = pow(in.core / rr, material.halo_falloff);

    var halo = profile * (1.0 - smoothstep(0.55, 1.0, r));
    if (material.corona_strength > 0.0 && length(in.sky) > 1e-6) {
        // Structure in the glare, not in the disc: the photosphere is smooth and the corona is
        // not. The sample direction leans along the line of sight as it goes out, so threads
        // evolve with distance instead of being perfectly straight spokes.
        let around = normalize(in.sky);
        let dir = normalize(around + in.axis * (r * 0.5));
        let threads = filaments(dir, in.seed);
        // How far this streamer goes, which is ragged rather than a circle. The fade has to
        // *finish* inside the quad: run it past r = 1 and the discard at the edge cuts it into
        // a hard disc, which is the circle this was meant to avoid, only sharper.
        let reach = material.corona_reach_min + reach_of(dir, in.seed) * material.corona_reach_span;
        let edge = 1.0 - smoothstep(reach, min(reach + material.corona_fade, 0.99), r);
        let lit = material.corona_floor + threads * material.corona_gain;
        halo = profile * edge * mix(1.0, lit, material.corona_strength);
    }

    // Alpha zero, and it has to be. AlphaMode::Add is premultiplied blending, `src + dst *
    // (1 - alpha)`, so an alpha of one is not addition -- it is a straight overwrite. With it,
    // every faint star's quad punched a dark square through the glare of a bright one, and
    // because two transparent meshes at the same depth are sorted with an arbitrary tie-break,
    // the squares flickered on and off from frame to frame. It read as z-fighting and it was
    // blend order. At alpha zero the blend is `src + dst` and the order stops mattering.
    return vec4<f32>(in.colour * (core + halo * material.halo_gain), 0.0);
}
