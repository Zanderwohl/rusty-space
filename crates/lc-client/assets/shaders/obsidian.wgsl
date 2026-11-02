// Obsidian: knapped volcanic glass, blue-gray-black, all reflection. The surface is cut by flake
// scars, cells of a 3D Voronoi on world position at two sizes, so any shape takes them without a
// seam. Each scar is a shallow concave dish with faint ripple rings and a tilt of its own, so two
// meet in a ridge and three in a peak. Look development: crates/lc-client/src/looks/obsidian.rs.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::{globals, view},
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
}

struct ObsidianUniform {
    /// `(scar_m, dish, tilt_rad, ripple)`: the larger scars' spacing, how far a dish's normal leans
    /// toward its middle at its rim, each scar's own tilt, and the ripples' lean.
    scar: vec4<f32>,
    /// The glass's color, linear, and its roughness in `w`.
    glass: vec4<f32>,
    /// Toward the star, and its turn in `w`.
    to_star: vec4<f32>,
    star_color: vec4<f32>,
    /// Direction to the planet's center, and its angular radius in `w`, radians.
    to_planet: vec4<f32>,
    planet_color: vec4<f32>,
    /// What the glass mirrors of the sky: dim above, dimmer below.
    sky: vec4<f32>,
    /// `(exposure, seed, 0, 0)`.
    finish: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: ObsidianUniform;

const PI: f32 = 3.14159265;
/// Obsidian's index is about 1.49.
const F0: f32 = 0.04;
/// The finer scars, as a share of the larger.
const FINE: f32 = 0.33;

/// The star, turned about world Y at `to_star.w` rad/s.
fn turned(to_star: vec4<f32>) -> vec3<f32> {
    let a = to_star.w * globals.time;
    let d = normalize(to_star.xyz);
    return vec3<f32>(cos(a) * d.x - sin(a) * d.z, d.y, sin(a) * d.x + cos(a) * d.z);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_position = world.xyz;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    return out;
}

fn pcg(x: u32) -> u32 {
    let state = x * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

fn cell_hash(c: vec3<i32>) -> u32 {
    return pcg(u32(c.x) ^ pcg(u32(c.y) ^ pcg(u32(c.z) ^ u32(m.finish.y))));
}

struct Scar {
    /// From the point to the scar's middle, meters.
    to_middle: vec3<f32>,
    /// How far the point is from the ridge with the next scar, meters.
    ridge: f32,
    id: u32,
}

/// The scar holding `p`, scars `size` meters apart.
fn scar(p: vec3<f32>, size: f32) -> Scar {
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
                let h = cell_hash(c);
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
/// dish's middle, rippled in rings about it, plus the scar's own tilt.
fn lean(s: Scar, normal: vec3<f32>, size: f32) -> vec3<f32> {
    let inward = s.to_middle - normal * dot(s.to_middle, normal);
    let r = length(inward);
    let toward = inward / max(r, 1.0e-4);
    let dish = m.scar.y * r / (0.5 * size);
    let ripple = m.scar.w * sin(r / size * 40.0);
    let a = 6.2831853 * unit(pcg(s.id + 7u));
    let own = vec3<f32>(cos(a), sin(a), cos(a + 1.3));
    let tilt = (own - normal * dot(own, normal)) * m.scar.z * unit(pcg(s.id + 9u));
    return toward * (dish + ripple) + tilt;
}

fn ggx(n: vec3<f32>, h: vec3<f32>, roughness: f32) -> f32 {
    let a2 = roughness * roughness * roughness * roughness;
    let nh = max(dot(n, h), 0.0);
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

/// Cook-Torrance with Smith-Schlick visibility and Schlick Fresnel, times n·l.
fn specular(n: vec3<f32>, l: vec3<f32>, e: vec3<f32>, roughness: f32) -> f32 {
    let h = normalize(l + e);
    let nl = max(dot(n, l), 0.0);
    let ne = max(dot(n, e), 1.0e-3);
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let vis = 1.0 / ((nl * (1.0 - k) + k) * (ne * (1.0 - k) + k) * 4.0);
    let fresnel = F0 + (1.0 - F0) * pow(1.0 - max(dot(h, e), 0.0), 5.0);
    return fresnel * ggx(n, h, roughness) * vis * nl;
}

/// The planet seen along `r`, its limb blurred by the roughness.
fn planet_seen(r: vec3<f32>, roughness: f32) -> f32 {
    let off = acos(clamp(dot(r, normalize(m.to_planet.xyz)), -1.0, 1.0));
    let blur = 0.5 * roughness * roughness + 0.01;
    return 1.0 - smoothstep(m.to_planet.w - blur, m.to_planet.w + blur, off);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let surface = normalize(in.world_normal);
    let p = in.world_position;
    let e = normalize(view.world_position - p);
    let size = m.scar.x;
    let pixel = max(length(fwidth(p)), 1.0e-4);

    // Scars smaller than a few pixels give up their shape to a rougher sheen, rather than sparkle.
    let coarse = scar(p, size);
    let fine = scar(p, size * FINE);
    let keep_coarse = 1.0 - smoothstep(0.15, 0.6, pixel / size);
    let keep_fine = 1.0 - smoothstep(0.15, 0.6, pixel / (size * FINE));
    let tangent = lean(coarse, surface, size) * keep_coarse + 0.25 * lean(fine, surface, size * FINE) * keep_fine;
    let n = normalize(surface + tangent);
    let roughness = mix(0.5, m.glass.w, keep_fine);

    // A ridge is sharp: the edge itself catches a thin line of light whichever way it faces.
    let edge = 1.0 - smoothstep(0.0, max(0.03 * size, pixel), coarse.ridge);
    let tint = m.glass.rgb * mix(0.8, 1.2, unit(pcg(coarse.id + 3u)));

    let ne = max(dot(n, e), 0.0);
    let fresnel = F0 + (1.0 - F0) * pow(1.0 - ne, 5.0);
    let r = reflect(-e, n);
    var radiance = m.sky.rgb * fresnel * mix(0.3, 1.0, 0.5 + 0.5 * r.y);
    radiance += m.planet_color.rgb * fresnel * planet_seen(r, roughness);
    let star = turned(m.to_star);
    if (dot(star, surface) > 0.0) {
        let ndl = max(dot(n, star), 0.0);
        radiance += m.star_color.rgb * (tint * ndl / PI + specular(n, star, e, roughness));
        radiance += m.star_color.rgb * 0.003 * edge * keep_coarse * max(dot(surface, star), 0.0);
    }
    return vec4<f32>(radiance * m.finish.x, 1.0);
}
