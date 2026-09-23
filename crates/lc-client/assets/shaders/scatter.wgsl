#define_import_path lightcone::scatter

// Single scattering in a body's air, with the body at the origin and lengths in its radius.
// Shared by the surface, which scatters it over the disc, and atmosphere.wgsl, which draws the
// limb. See lightcone/docs/07-rendering.md, "Air".

const PI: f32 = 3.14159265;

/// Scale heights to the top of the air; em_render::atmosphere_material::TOP_HEIGHTS.
const TOP_HEIGHTS: f32 = 6.0;
const VIEW_STEPS: i32 = 16;
const STAR_STEPS: i32 = 6;
/// Henyey-Greenstein asymmetry of haze: dust and droplets throw light forward.
const HAZE_G: f32 = 0.6;
/// Share of the haze's phase that is isotropic instead. It stands in for the multiple
/// scattering a single-scattering march loses, without which a dusty limb reads darker than
/// the ground under it.
const HAZE_DIFFUSE: f32 = 0.4;
/// Scale heights below the ground past which a ray counts as through the body. Only to keep
/// the exponential finite: the shadow it makes is already black a few heights down.
const DEEPEST: f32 = 60.0;

/// Every depth is per display channel: the host averages each band's over the bands the mapping
/// puts on that channel, so a channel carrying K sees through Rayleigh.
struct Air {
    /// Vertical optical depth of the gas.
    gas: vec3<f32>,
    /// Scale height, radii.
    height: f32,
    /// Vertical optical depth of the haze.
    haze: vec3<f32>,
    /// Vertical optical depth at ten microns, absorbing.
    infrared: f32,
    haze_albedo: vec3<f32>,
}

/// From the uniforms' packing: `gas.w` is the height and `haze.w` the infrared depth.
fn air_of(gas: vec4<f32>, haze: vec4<f32>, albedo: vec4<f32>) -> Air {
    return Air(gas.xyz, gas.w, haze.xyz, haze.w, albedo.xyz);
}

fn top_of(air: Air) -> f32 {
    return 1.0 + TOP_HEIGHTS * air.height;
}

/// Where a ray from `o` along unit `d` enters and leaves a sphere of radius `r` at the origin;
/// the second below the first on a miss. Written about the closest approach rather than as the
/// textbook quadratic, whose `b * b - c` cancels to nothing from a thousand radii out.
fn crossing(o: vec3<f32>, d: vec3<f32>, r: f32) -> vec2<f32> {
    let b = dot(o, d);
    let closest = o - b * d;
    let h = r * r - dot(closest, closest);
    if (h < 0.0) {
        return vec2<f32>(1.0, -1.0);
    }
    let s = sqrt(h);
    return vec2<f32>(-b - s, -b + s);
}

fn density(p: vec3<f32>, air: Air) -> f32 {
    return exp(-(length(p) - 1.0) / air.height);
}

/// Per radius, at the surface's density.
fn extinction(air: Air) -> vec3<f32> {
    return (air.gas + air.haze) / air.height;
}

/// Optical depth from `p` out toward the star along `l`.
///
/// With `occlude` a ray through the body keeps descending, into air the exponential makes as
/// dense as it likes, which is the body's shadow with a penumbra. A hard test instead lights
/// each sample of a march wholly or not at all, and the terminator comes out in steps, one a
/// sample. Without it, the ground just past the terminator keeps the soft edge the surface
/// gives it.
fn star_depth(p: vec3<f32>, l: vec3<f32>, air: Air, occlude: bool) -> vec3<f32> {
    let out = crossing(p, l, top_of(air)).y;
    if (out <= 0.0) {
        return vec3<f32>(0.0);
    }
    let ds = out / f32(STAR_STEPS);
    var sum = 0.0;
    for (var i = 0; i < STAR_STEPS; i++) {
        let q = p + l * ((f32(i) + 0.5) * ds);
        var below = (1.0 - length(q)) / air.height;
        if (!occlude) {
            below = min(below, 0.0);
        }
        sum += exp(min(below, DEEPEST));
    }
    return extinction(air) * sum * ds;
}

struct Scattered {
    /// In units of the starlight on a white surface facing the star.
    light: vec3<f32>,
    /// What survives of whatever is behind.
    through: vec3<f32>,
    /// How much air the ray crossed, in vertical columns: what ten microns' absorption reads.
    column: f32,
}

/// Along `d` from `t0` to `t1`, lit from `l`.
fn scatter(o: vec3<f32>, d: vec3<f32>, t0: f32, t1: f32, l: vec3<f32>, air: Air) -> Scattered {
    var out: Scattered;
    out.light = vec3<f32>(0.0);
    out.through = vec3<f32>(1.0);
    out.column = 0.0;
    if (t1 <= t0 || air.height <= 0.0) {
        return out;
    }
    let mu = dot(d, l);
    let rayleigh = 3.0 / (16.0 * PI) * (1.0 + mu * mu);
    let g2 = HAZE_G * HAZE_G;
    let hg = mix((1.0 - g2) / (4.0 * PI * pow(1.0 + g2 - 2.0 * HAZE_G * mu, 1.5)), 1.0 / (4.0 * PI), HAZE_DIFFUSE);
    let phased = (air.gas * rayleigh + air.haze * air.haze_albedo * hg) / air.height;
    let ext = extinction(air);
    let ds = (t1 - t0) / f32(VIEW_STEPS);
    var depth = vec3<f32>(0.0);
    for (var i = 0; i < VIEW_STEPS; i++) {
        let p = o + d * (t0 + (f32(i) + 0.5) * ds);
        let rho = density(p, air);
        let step = ext * rho * ds;
        let lit = exp(-(depth + 0.5 * step) - star_depth(p, l, air, true));
        out.light += phased * rho * ds * lit;
        depth += step;
        out.column += rho * ds;
    }
    out.column /= air.height;
    // Starlight on a white Lambertian surface is the irradiance over pi.
    out.light *= PI;
    out.through = exp(-depth);
    return out;
}
