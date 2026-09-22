// The observer's sky: one camera-facing quad per star, shaded from physics.
//
// Descended from assets/shaders/starfield.wgsl in the Exotic Matters app, which established
// the technique: bake four vertices per star into one mesh, expand the quad in view space in
// the vertex stage, and clamp to background depth. What is different here is that nothing
// about a star's *appearance* is baked. The catalogue shader bakes a color and a brightness
// because its only input is an apparent magnitude; this one bakes a temperature and a radius
// and derives the rest per frame, because aberration, Doppler shift, the band mapping and the
// exposure all change while the ship flies and re-uploading the mesh for them does not scale.
//
// Per-vertex, baked:
//   POSITION : star position relative to the bake origin, light-years, render axes
//   CORNER   : quad corner in [-1, 1]^2, also the radial falloff coordinate
//   PARAMS   : (effective temperature K, radius m, corona seed, unused)
//   WARM     : (population temperature K, its radiance over the star's disc, gray deficit, -)
//
// Everything else is a uniform. See lightcone/docs/07-rendering.md.

#import bevy_pbr::mesh_view_bindings::view

const PI: f32 = 3.14159265;
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
const BANDS: u32 = 7u;
/// In the corona's reach. Must match `CORONA_FLOW_CYCLE` in em-render.
const CORONA_FLOW_CYCLE: f32 = 0.5;

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
    /// How far the corona reaches, in **stellar radii**.
    ///
    /// The one quantity here that is a world size rather than a screen size. Everything else
    /// about a point source is angular on purpose — a star's glare is an artifact of looking at
    /// it, and does not grow as you approach. A corona is a thing that is *there*, so its
    /// angular size has to fall off with distance like the disc it surrounds.
    corona_radii: f32,
    /// How far through its cycle the outward drift is, in [0, 1).
    corona_flow_phase: f32,
    // Lookup domain: index = (log2(T) - log_t_min) * log_t_scale.
    log_t_min: f32,
    log_t_scale: f32,
    lut_samples: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: StarfieldUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var band_lut: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var corona_filaments: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var filaments_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var corona_reach: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var reach_sampler: sampler;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) corner: vec2<f32>,
    @location(2) params: vec4<f32>,
    @location(3) warm: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) color: vec3<f32>,
    /// Where the source itself ends and the glare begins, as a fraction of the quad.
    @location(2) core: f32,
    /// Offset from the star in the plane of the sky, world axes. Its *direction* is the angle
    /// around the star; its length is the distance out.
    @location(3) sky: vec3<f32>,
    /// World direction to the star, the axis the corona is arranged about.
    @location(4) axis: vec3<f32>,
    /// The star's own turn of the corona, as the columns of a rotation. See `spin_of`.
    @location(5) @interpolate(flat) spin_x: vec3<f32>,
    /// How much of the quad the corona fills. The rest of the quad is glare, which is angular.
    @location(6) corona: f32,
    @location(7) @interpolate(flat) spin_y: vec3<f32>,
    @location(8) @interpolate(flat) spin_z: vec3<f32>,
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
//
// Both fields are baked from `textures/corona.tgraph` onto cubemaps, once, for every star.

/// A rotation drawn from a star's seed, which turns the one baked corona a different way for
/// every star. Where the shader used to offset the noise by the seed, this turns it: as
/// distinct, and still a function of the seed and the world direction only.
///
/// `fract` before the trigonometry keeps every argument small, so every GPU computes the same
/// turn.
fn spin_of(seed: f32) -> mat3x3<f32> {
    let azimuth = fract(seed * 0.7548777) * 2.0 * PI;
    // Uniform in z is uniform over the sphere, so no axis is favoured.
    let z = fract(seed * 0.5698403) * 2.0 - 1.0;
    let angle = fract(seed * 0.3819660) * 2.0 * PI;
    let s = sqrt(max(1.0 - z * z, 0.0));
    let k = vec3<f32>(s * cos(azimuth), s * sin(azimuth), z);
    let c = cos(angle);
    let n = sin(angle);
    let t = 1.0 - c;
    return mat3x3<f32>(
        vec3<f32>(c + t * k.x * k.x, t * k.x * k.y + n * k.z, t * k.x * k.z - n * k.y),
        vec3<f32>(t * k.x * k.y - n * k.z, c + t * k.y * k.y, t * k.y * k.z + n * k.x),
        vec3<f32>(t * k.x * k.z + n * k.y, t * k.y * k.z - n * k.x, c + t * k.z * k.z),
    );
}

/// The corona's threads, carried outward as the drift phase advances.
///
/// A thread's pattern changes with distance out only through the lean along the line of sight,
/// so sliding the lean back as time runs moves every feature outward along its thread. A single
/// slide has to jump back at the end of its cycle; two copies half a cycle apart, each faded to
/// nothing at its own jump, hide it. The weights sum to one.
fn drifting_threads(spin: mat3x3<f32>, around: vec3<f32>, axis: vec3<f32>, out_by: f32) -> f32 {
    var sum = 0.0;
    for (var k = 0u; k < 2u; k = k + 1u) {
        let phase = fract(material.corona_flow_phase + 0.5 * f32(k));
        let weight = 1.0 - abs(2.0 * phase - 1.0);
        let lean = out_by - phase * CORONA_FLOW_CYCLE;
        let dir = spin * normalize(around + axis * (lean * 0.5));
        // Level zero: the caller's branch is not uniform control flow, so no implicit derivative.
        sum = sum + weight * textureSampleLevel(corona_filaments, filaments_sampler, dir, 0.0).r;
    }
    return sum;
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
    // meters squared overflows f32 past a couple of thousand light-years, and the ratio never
    // does.
    let disc_rad = vertex.params.y / (distance_ly * 9.4607305e15);
    let geometry = PI * disc_rad * disc_rad;

    // A population absorbs starlight and re-emits it as a blackbody at the temperature its
    // orbit sets. Occultation is gray and removes; re-emission is cold and adds, which in the
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
    out.color = chroma * material.brightness * (level + glow * material.overflow_gain);

    // The source itself: its disc if that is resolvable, otherwise the smallest thing worth
    // drawing. At sixty astronomical units a sun is a fiftieth of a pixel across, and it is
    // bright rather than big.
    let core_rad = max(disc_rad, material.min_radius_rad);
    // The glare around it, which is what grows with brightness.
    let glare_rad = mix(material.min_radius_rad, material.max_radius_rad, level)
        + material.glow_radius_gain * glow * material.min_radius_rad;

    // The corona, which is a world size: a fixed number of stellar radii, so it subtends less
    // as the ship draws away and more as it closes, exactly as the disc does. Tying it to the
    // glare instead left it the same size on screen at every distance — huge from far off and
    // a tight collar up close.
    let corona_rad = disc_rad * material.corona_radii;

    // The quad covers whichever is largest. One quad drawing a single filled disc made a star
    // a hundred and sixty pixels across into a flat white ball, with the disc it actually has
    // swamped inside it.
    let radius_rad = max(max(core_rad, glare_rad), corona_rad);
    out.core = clamp(core_rad / radius_rad, 0.0, 1.0);
    // What fraction of the quad the corona is allowed to fill.
    out.corona = clamp(corona_rad / radius_rad, 0.0, 1.0);

    // Offset from the star in the plane of the sky, in world axes so the pattern is anchored
    // to the star rather than to the camera. Normalising this in the fragment discards the
    // distance out and leaves only the angle around the star, which is what makes every
    // feature a radial thread instead of a blob.
    //
    // The quad lies in the *screen* plane, which is square to the line of sight only for a star
    // dead ahead. Off center the camera's right and up carry a component along the line to the
    // star, and the lean in the fragment turned that into a different corona wherever the star
    // sat in the frame: orbiting the camera a few hundred meters round the ship reshaped it.
    // With that component removed the pattern depends on the line from star to ship alone.
    let right_world = (view.world_from_view * vec4<f32>(1.0, 0.0, 0.0, 0.0)).xyz;
    let up_world = (view.world_from_view * vec4<f32>(0.0, 1.0, 0.0, 0.0)).xyz;
    let across = right_world * vertex.corner.x + up_world * vertex.corner.y;
    out.sky = across - seen * dot(across, seen);
    out.axis = seen;
    let spin = spin_of(vertex.params.z);
    out.spin_x = spin[0];
    out.spin_y = spin[1];
    out.spin_z = spin[2];

    // w = 0 drops the camera's translation, so the sky depends only on where it is pointed.
    // The quad center sits one unit down the view ray, which makes the corner offset equal to
    // the angle subtended, so radius_rad is an angular radius with no projection arithmetic.
    let dir_view = normalize((view.view_from_world * vec4<f32>(seen, 0.0)).xyz);
    let pos_view = dir_view + vec3<f32>(vertex.corner, 0.0) * radius_rad;
    var clip = view.clip_from_view * vec4<f32>(pos_view, 1.0);
    // Just in front of the far plane, so the sky is behind everything real.
    //
    // Depth is reversed here: `clip.z = clip.w` is the *near* plane, not the far one, and with
    // nothing else in the scene that never showed. The moment a planet wrote depth, the stars
    // came out in front of it. Not zero either -- the buffer is cleared to zero and the test
    // is a strict greater-than, so a star at exactly zero fails everywhere.
    //
    // The value has to stay under the smallest a real body produces. Reversed depth is
    // near/z, and the near plane is 1e-10 render units against an Oort cloud at 1e5, so the
    // faintest real depth is of order 1e-15. Anything above that and the sky punches through
    // the outer system.
    clip.z = clip.w * 1.0e-20;
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
        // evolve with distance instead of being perfectly straight spokes. The lean is measured
        // in the corona's own reach rather than the quad's, which the glare sizes by exposure.
        let around = normalize(in.sky);
        let out_by = r / max(in.corona, 1e-6);
        let spin = mat3x3<f32>(in.spin_x, in.spin_y, in.spin_z);
        let dir = spin * normalize(around + in.axis * (out_by * 0.5));
        // The threads drift and the silhouette does not: a streamer's tips flickering as two
        // copies crossfade would read as noise rather than gas going somewhere.
        let threads = drifting_threads(spin, around, in.axis, out_by);
        // How far this streamer goes, which is ragged rather than a circle. The fade has to
        // *finish* inside the quad: run it past r = 1 and the discard at the edge cuts it into
        // a hard disc, which is the circle this was meant to avoid, only sharper.
        // Scaled into the corona's own share of the quad, so a streamer's tip is a fixed
        // distance from the star in the world rather than a fixed fraction of the sprite.
        let reach_here = textureSampleLevel(corona_reach, reach_sampler, dir, 0.0).r;
        let reach = (material.corona_reach_min + reach_here * material.corona_reach_span)
            * in.corona;
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
    return vec4<f32>(in.color * (core + halo * material.halo_gain), 0.0);
}
