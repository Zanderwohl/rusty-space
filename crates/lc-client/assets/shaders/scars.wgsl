// Flake scars, as knapped obsidian has: cells of a 3D Voronoi on a position, at two sizes, each a
// shallow concave dish with faint ripple rings and a tilt of its own, so two meet in a ridge and
// three in a peak. Shared by obsidian.wgsl and hull.wgsl, which take positions in their own frames.

fn pcg(x: u32) -> u32 {
    let state = x * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

struct Scar {
    /// From the point to the scar's middle, meters.
    to_middle: vec3<f32>,
    /// How far the point is from the ridge with the next scar, meters.
    ridge: f32,
    id: u32,
}

/// The scar holding `p`, scars `size` meters apart.
fn scar(p: vec3<f32>, size: f32, seed: u32) -> Scar {
    let x = p / size;
    let i = vec3<i32>(floor(x));
    var best = 1.0e9;
    var second = 1.0e9;
    var middle = vec3<f32>(0.0);
    var id = 0u;
    for (var dz = -1; dz <= 1; dz++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let c = i + vec3<i32>(dx, dy, dz);
                let h = pcg(u32(c.x) ^ pcg(u32(c.y) ^ pcg(u32(c.z) ^ seed)));
                let at = vec3<f32>(c) + vec3<f32>(unit(pcg(h)), unit(pcg(h + 1u)), unit(pcg(h + 2u)));
                let d = dot(at - x, at - x);
                if (d < best) {
                    second = best;
                    best = d;
                    middle = at;
                    id = h;
                } else if (d < second) {
                    second = d;
                }
            }
        }
    }
    // Half the gap between the two nearest distances is near enough the distance to their bisector.
    return Scar((middle - x) * size, 0.5 * (sqrt(second) - sqrt(best)) * size, id);
}

/// The lean a scar gives the surface at a point, in the tangent plane of `normal`: toward the
/// dish's middle, rippled in rings about it, plus the scar's own tilt. `shape` is `(dish, tilt,
/// ripple)`, each a tangent: the dish's lean at its rim, the most a scar tilts, the ripples' lean.
fn lean(s: Scar, normal: vec3<f32>, size: f32, shape: vec3<f32>) -> vec3<f32> {
    let inward = s.to_middle - normal * dot(s.to_middle, normal);
    let r = length(inward);
    let toward = inward / max(r, 1.0e-4);
    let dish = shape.x * r / (0.5 * size);
    let ripple = shape.z * sin(r / size * 40.0);
    let a = 6.2831853 * unit(pcg(s.id + 7u));
    let own = vec3<f32>(cos(a), sin(a), cos(a + 1.3));
    let tilt = (own - normal * dot(own, normal)) * shape.y * unit(pcg(s.id + 9u));
    return toward * (dish + ripple) + tilt;
}

/// The finer scars, as a share of the larger.
const FINE: f32 = 0.33;

struct Knapped {
    normal: vec3<f32>,
    /// What is left of each size's shape once a pixel spans it: under a few pixels a scar gives up
    /// its shape to a rougher sheen, rather than sparkle.
    keep_coarse: f32,
    keep_fine: f32,
    coarse: Scar,
}

/// `normal` knapped at `p` by scars `size` apart and a third of that, `pixel` meters across.
fn knapped(p: vec3<f32>, normal: vec3<f32>, size: f32, shape: vec3<f32>, pixel: f32, seed: u32) -> Knapped {
    let coarse = scar(p, size, seed);
    let fine = scar(p, size * FINE, seed + 1u);
    let keep_coarse = 1.0 - smoothstep(0.15, 0.6, pixel / size);
    let keep_fine = 1.0 - smoothstep(0.15, 0.6, pixel / (size * FINE));
    let tangent = lean(coarse, normal, size, shape) * keep_coarse + 0.25 * lean(fine, normal, size * FINE, shape) * keep_fine;
    return Knapped(normalize(normal + tangent), keep_coarse, keep_fine, coarse);
}

/// Cook-Torrance with GGX, Smith-Schlick visibility and Schlick Fresnel at `f0`, times n·l.
fn glass_specular(n: vec3<f32>, l: vec3<f32>, e: vec3<f32>, roughness: f32, f0: f32) -> f32 {
    let h = normalize(l + e);
    let nl = max(dot(n, l), 0.0);
    let ne = max(dot(n, e), 1.0e-3);
    let a2 = roughness * roughness * roughness * roughness;
    let nh = max(dot(n, h), 0.0);
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let ggx = a2 / (3.14159265 * dd * dd);
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let vis = 1.0 / ((nl * (1.0 - k) + k) * (ne * (1.0 - k) + k) * 4.0);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - max(dot(h, e), 0.0), 5.0);
    return fresnel * ggx * vis * nl;
}
