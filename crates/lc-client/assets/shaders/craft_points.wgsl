// Craft too far to resolve, and collapses: one camera-facing quad each, shaded as a sum of
// blackbodies and lines. See em_render::craft_point_material and lightcone/docs/32-ship-rendering.md
// §The exhaust cone, From a distance.
//
// The star path of starfield.wgsl, which this must stay in step with: the same aberration,
// Doppler, band table and tone map, so a point is exposed as a star is. Without the corona and the
// population, which a craft has neither of.
//
// Per-vertex:
//   POSITION : relative to the bake origin, light-years, render axes
//   CORNER   : quad corner in [-1, 1]^2
//   TERMS_A  : (kelvin, sr, kelvin, sr); a negative kelvin is a line at that many meters, flux in sr,
//              and FLAT is the same flux in every band
//   TERMS_B  : two more
//   CLOCK    : (start, flash, fade, wrap), real seconds as globals.time reads them; start < 0: none
//   EVENT    : (flash kelvin, flash sr, fade's first kelvin, fade's sr), the flash a term as above

#import bevy_pbr::mesh_view_bindings::{view, globals}

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
const BANDS: u32 = 7u;
/// distant::FLAT: a term the same in every band.
const FLAT: f32 = -1.0e38;
/// em_spectra's band limits, meters. A test holds these to it.
const BAND_LO: array<f32, 7> = array<f32, 7>(3.98e-7, 5.07e-7, 5.89e-7, 7.315e-7, 1.995e-6, 7.5e-6, 0.21099);
const BAND_HI: array<f32, 7> = array<f32, 7>(4.92e-7, 5.95e-7, 7.27e-7, 8.805e-7, 2.385e-6, 1.25e-5, 0.21114);

struct StarfieldUniform {
    band_to_display: array<vec4<f32>, 7>,
    beta: vec4<f32>,
    ship_offset_ly: vec4<f32>,
    reference: f32,
    point_stops: f32,
    min_radius_rad: f32,
    max_radius_rad: f32,
    glow_radius_gain: f32,
    brightness: f32,
    overflow_gain: f32,
    halo_gain: f32,
    corona_strength: f32,
    halo_falloff: f32,
    corona_reach_min: f32,
    corona_reach_span: f32,
    corona_fade: f32,
    corona_floor: f32,
    corona_gain: f32,
    corona_radii: f32,
    corona_flow_phase: f32,
    log_t_min: f32,
    log_t_scale: f32,
    lut_samples: f32,
    drawn_rad_per_px: f32,
    drawn_exposure: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: StarfieldUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var band_lut: texture_2d<f32>;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) corner: vec2<f32>,
    @location(2) terms_a: vec4<f32>,
    @location(3) terms_b: vec4<f32>,
    @location(4) clock: vec4<f32>,
    @location(5) event: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) color: vec3<f32>,
    @location(2) core: f32,
};

fn lorentz(beta: vec3<f32>) -> f32 {
    return inverseSqrt(max(1.0 - dot(beta, beta), 1e-12));
}

fn aberrate(to_source: vec3<f32>, beta: vec3<f32>) -> vec3<f32> {
    let n = -to_source;
    let g = lorentz(beta);
    let ndb = dot(n, beta);
    let num = n + beta * (g * (g / (g + 1.0) * ndb - 1.0));
    return -normalize(num / (g * (1.0 - ndb)));
}

fn doppler(to_source: vec3<f32>, beta: vec3<f32>) -> f32 {
    return lorentz(beta) * (1.0 + dot(beta, to_source));
}

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

/// One term's flux in `band` for an observer whose Doppler factor toward it is `d`. A blackbody seen
/// at `d` is one at `d` times the temperature; a line moves to its wavelength over `d` and, as the
/// blackbody's total does, carries `d⁴` of its flux.
fn term(band: u32, kelvin: f32, sr: f32, d: f32) -> f32 {
    if (sr <= 0.0) {
        return 0.0;
    }
    if (kelvin > 0.0) {
        return band_radiance(band, max(kelvin * d, 1.0)) * sr;
    }
    if (kelvin <= FLAT) {
        return sr * d * d * d * d;
    }
    let wavelength = -kelvin / d;
    if (wavelength >= BAND_LO[band] && wavelength <= BAND_HI[band]) {
        return sr * d * d * d * d;
    }
    return 0.0;
}

fn pixel_scale() -> f32 {
    if (material.drawn_rad_per_px <= 0.0) {
        return 1.0;
    }
    let here = 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    return here / material.drawn_rad_per_px;
}

fn exposure_gain() -> f32 {
    if (material.drawn_exposure <= 0.0) {
        return 1.0;
    }
    return view.exposure / material.drawn_exposure;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.corner = vertex.corner;

    let rel = vertex.position - material.ship_offset_ly.xyz;
    let to_source = rel / max(length(rel), 1e-12);
    let beta = material.beta.xyz;
    let seen = aberrate(to_source, beta);
    let d = doppler(to_source, beta);

    // The event, if it is playing: the flash, then the fade. Unwrapped once: an event outlasting
    // globals.time's wrap period is not drawn by this.
    var flash_sr = 0.0;
    var fade_k = 0.0;
    var fade_sr = 0.0;
    if (vertex.clock.x >= 0.0) {
        var t = globals.time - vertex.clock.x;
        if (t < 0.0) {
            t = t + vertex.clock.w;
        }
        if (t < vertex.clock.y) {
            flash_sr = vertex.event.y;
        }
        let tau = t / max(vertex.clock.z, 1e-6);
        if (tau < 1.0) {
            fade_k = vertex.event.z * sqrt(sqrt(1.0 - tau));
            fade_sr = vertex.event.w;
        }
    }

    var linear = vec3<f32>(0.0);
    for (var b = 0u; b < BANDS; b = b + 1u) {
        let flux = term(b, vertex.terms_a.x, vertex.terms_a.y, d)
            + term(b, vertex.terms_a.z, vertex.terms_a.w, d)
            + term(b, vertex.terms_b.x, vertex.terms_b.y, d)
            + term(b, vertex.terms_b.z, vertex.terms_b.w, d)
            + term(b, vertex.event.x, flash_sr, d)
            + term(b, fade_k, fade_sr, d);
        linear = linear + material.band_to_display[b].rgb * flux;
    }

    // starfield.wgsl's tone map.
    let luminance = dot(linear, LUMA);
    let peak = max(linear.r, max(linear.g, linear.b));
    var above = -1e9;
    if (luminance > 0.0 && material.reference > 0.0) {
        above = log2(luminance * exposure_gain() / material.reference);
    }
    let chroma = select(vec3<f32>(1.0), linear / peak, peak > 0.0);
    let level = clamp(1.0 + above / max(material.point_stops, 1e-6), 0.0, 1.0);
    let glow = clamp(above, 0.0, 24.0);
    out.color = chroma * material.brightness * (level + glow * material.overflow_gain);

    let scale = pixel_scale();
    let min_rad = material.min_radius_rad * scale;
    let glare_rad = mix(min_rad, material.max_radius_rad * scale, level)
        + material.glow_radius_gain * glow * min_rad;
    let radius_rad = max(min_rad, glare_rad);
    out.core = clamp(min_rad / radius_rad, 0.0, 1.0);

    let dir_view = normalize((view.view_from_world * vec4<f32>(seen, 0.0)).xyz);
    let pos_view = dir_view + vec3<f32>(vertex.corner, 0.0) * radius_rad;
    var clip = view.clip_from_view * vec4<f32>(pos_view, 1.0);
    // Behind everything real, as a star is; reversed depth, so not zero.
    clip.z = clip.w * 1.0e-20;
    // Nothing to draw, or behind the camera.
    if (dir_view.z > 0.0 || luminance <= 0.0) {
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
    let core = 1.0 - smoothstep(in.core * 0.8, in.core, r);
    let rr = max(r, in.core);
    let halo = pow(in.core / rr, material.halo_falloff) * (1.0 - smoothstep(0.55, 1.0, r));
    // Alpha zero: AlphaMode::Add is premultiplied, and one would overwrite.
    return vec4<f32>(in.color * (core + halo * material.halo_gain), 0.0);
}
