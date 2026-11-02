// Azteced sheet metal: rectangles cut from a staggered grid by random splits, each panel one
// color and one flat normal tipped a few degrees, so panels catch the light one by one. Fully
// procedural, so nothing repeats. Each face is mapped in its own frame (u, v, n), meters: the
// mesh must be axis-aligned boxes. Look development: crates/lc-client/examples/aztec_void.rs.

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
}

struct AztecUniform {
    /// `(cell_m, min_panel_m, snap_m, 0)`.
    grid: vec4<f32>,
    /// `(width_m, lean_rad, 0, 0)`: each wall of a seam's trench.
    chamfer: vec4<f32>,
    /// `(tilt_rad, brown_share, roughness, metallic)`.
    panel: vec4<f32>,
    to_star: vec4<f32>,
    star_color: vec4<f32>,
    /// Direction to the planet's center, and its angular radius in `w`, radians.
    to_planet: vec4<f32>,
    planet_color: vec4<f32>,
    ambient: vec4<f32>,
    /// `(exposure, seed, 0, 0)`.
    finish: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: AztecUniform;

const PI: f32 = 3.14159265;
const SPLITS: u32 = 6u;

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

struct Face {
    u: vec3<f32>,
    v: vec3<f32>,
    n: vec3<f32>,
    id: u32,
}

fn face(normal: vec3<f32>) -> Face {
    let n = normalize(normal);
    let up = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.z) > 0.9);
    let u = normalize(cross(up, n));
    return Face(u, cross(n, u), n, u32(dot(round(n), vec3<f32>(1.0, 3.0, 9.0)) + 13.0));
}

fn pcg(x: u32) -> u32 {
    let state = x * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn mix_hash(a: u32, b: u32) -> u32 {
    return pcg(a ^ (b + 0x9e3779b9u + (a << 6u) + (a >> 2u)));
}

fn unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

/// Grid snapped coordinates are exact, so a panel's bounds name it whichever pixel asks.
fn snapped_id(x: f32) -> u32 {
    return u32(i32(round(x / m.grid.z)));
}

struct Panel {
    lo: vec2<f32>,
    hi: vec2<f32>,
    id: u32,
}

/// The panel holding `p`. Cells are staggered by row so the grid's lines do not run the face's
/// length, then split on a snapped random line, mostly across the longer side, until a split
/// would leave a panel under `min_panel_m` or the hash says stop.
fn panel(p: vec2<f32>, face_id: u32) -> Panel {
    let cell = m.grid.x;
    let snap = m.grid.z;
    let seed = mix_hash(face_id, u32(m.finish.y));
    let row = floor(p.y / cell);
    let shift = round(unit(mix_hash(seed, u32(i32(row)))) * cell / snap) * snap;
    let col = floor((p.x - shift) / cell);
    var lo = vec2<f32>(col * cell + shift, row * cell);
    var hi = lo + vec2<f32>(cell);
    var id = mix_hash(mix_hash(seed, u32(i32(row))), u32(i32(col)));
    for (var depth = 0u; depth < SPLITS; depth++) {
        let h = pcg(id);
        let size = hi - lo;
        if (depth >= 1u && unit(h) < 0.22 * f32(depth)) {
            break;
        }
        let longer_x = size.x >= size.y;
        let across_x = select(!longer_x, longer_x, unit(pcg(h)) < 0.8);
        let span = select(size.y, size.x, across_x);
        if (span < 2.0 * m.grid.y) {
            break;
        }
        let share = mix(0.2, 0.8, unit(pcg(h + 1u)));
        let at = select(lo.y, lo.x, across_x) + max(round(share * span / snap) * snap, m.grid.y);
        let cut = min(at, select(hi.y, hi.x, across_x) - m.grid.y);
        if (across_x) {
            if (p.x < cut) { hi.x = cut; } else { lo.x = cut; }
        } else {
            if (p.y < cut) { hi.y = cut; } else { lo.y = cut; }
        }
        id = mix_hash(mix_hash(snapped_id(lo.x) ^ (snapped_id(lo.y) << 16u), snapped_id(hi.x) ^ (snapped_id(hi.y) << 16u)), seed);
    }
    return Panel(lo, hi, id);
}

/// The panel's normal in the face's frame: tipped by up to `tilt_rad`, uniform over that disc.
fn tipped(id: u32) -> vec3<f32> {
    let r = m.panel.x * sqrt(unit(pcg(id + 7u)));
    let a = 2.0 * PI * unit(pcg(id + 11u));
    return vec3<f32>(sin(r) * cos(a), sin(r) * sin(a), cos(r));
}

/// Mostly gray, a few browner, every panel a shade off its neighbors.
fn paint(id: u32) -> vec3<f32> {
    let gray = mix(0.36, 0.46, unit(pcg(id + 3u)));
    let brown = unit(pcg(id + 5u)) < m.panel.y;
    let tint = select(vec3<f32>(1.0), mix(vec3<f32>(1.0), vec3<f32>(1.06, 0.95, 0.82), 0.5 + 0.5 * unit(pcg(id + 9u))), brown);
    return gray * tint;
}

fn ggx(n: vec3<f32>, h: vec3<f32>, roughness: f32) -> f32 {
    let a2 = roughness * roughness * roughness * roughness;
    let nh = max(dot(n, h), 0.0);
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

/// Cook-Torrance with Smith-Schlick visibility and Schlick Fresnel, times n·l.
fn specular(n: vec3<f32>, l: vec3<f32>, e: vec3<f32>, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    let h = normalize(l + e);
    let nl = max(dot(n, l), 0.0);
    let ne = max(dot(n, e), 1.0e-3);
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let vis = 1.0 / ((nl * (1.0 - k) + k) * (ne * (1.0 - k) + k) * 4.0);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - max(dot(h, e), 0.0), 5.0);
    return fresnel * ggx(n, h, roughness) * vis * nl;
}

/// The planet seen along `r`, its limb blurred by the panel's roughness: what makes a tipped
/// panel read as a different panel when the planet is behind the camera.
fn planet_seen(r: vec3<f32>, roughness: f32) -> f32 {
    let off = acos(clamp(dot(r, normalize(m.to_planet.xyz)), -1.0, 1.0));
    let blur = 0.5 * roughness * roughness + 0.01;
    return 1.0 - smoothstep(m.to_planet.w - blur, m.to_planet.w + blur, off);
}

/// What a surface of normal `n` sends toward the eye `e`.
fn lit(n: vec3<f32>, e: vec3<f32>, face_n: vec3<f32>, albedo: vec3<f32>, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    var radiance = m.ambient.rgb * albedo;

    let star = normalize(m.to_star.xyz);
    if (dot(star, face_n) > 0.0) {
        let ndl = max(dot(n, star), 0.0);
        radiance += m.star_color.rgb * (albedo * ndl / PI + specular(n, star, e, f0, roughness));
    }

    // A disc of uniform radiance: diffuse by its solid angle, specular by what the mirror sees.
    let to_planet = normalize(m.to_planet.xyz);
    let solid = 2.0 * PI * (1.0 - cos(m.to_planet.w));
    radiance += m.planet_color.rgb * albedo * max(dot(n, to_planet), 0.0) * solid / PI;
    let ne = max(dot(n, e), 0.0);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - ne, 5.0);
    radiance += m.planet_color.rgb * fresnel * planet_seen(reflect(-e, n), roughness);
    return radiance;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let f = face(in.world_normal);
    let p = vec2<f32>(dot(in.world_position, f.u), dot(in.world_position, f.v));
    let cell = panel(p, f.id);

    let local = tipped(cell.id);
    let n = normalize(f.u * local.x + f.v * local.y + f.n * local.z);
    let base = paint(cell.id);
    let roughness = m.panel.z;
    let metallic = m.panel.w;
    let albedo = base * (1.0 - metallic);
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let e = normalize(view.world_position - in.world_position);

    // A chamfer slopes down into the seam from each side, so the two walls of the trench face
    // each other. Its normal leans toward the nearest edge, which miters the corners.
    let to_edge = vec4<f32>(p.x - cell.lo.x, cell.hi.x - p.x, p.y - cell.lo.y, cell.hi.y - p.y);
    let edge = min(min(to_edge.x, to_edge.y), min(to_edge.z, to_edge.w));
    var outward = vec2<f32>(0.0, 1.0);
    if (edge == to_edge.x) { outward = vec2<f32>(-1.0, 0.0); }
    else if (edge == to_edge.y) { outward = vec2<f32>(1.0, 0.0); }
    else if (edge == to_edge.z) { outward = vec2<f32>(0.0, -1.0); }
    let lean = m.chamfer.y;
    let wall = normalize(n * cos(lean) + (f.u * outward.x + f.v * outward.y) * sin(lean));

    // Where a pixel spans the wall, its share of the pixel stands in for it.
    let pixel = max(length(fwidth(p)), 1.0e-4);
    let width = m.chamfer.x;
    let share = (1.0 - smoothstep(width - pixel, width + pixel, edge)) * clamp(width / pixel, 0.0, 1.0);

    let flat_light = lit(n, e, f.n, albedo, f0, roughness);
    let wall_light = lit(wall, e, f.n, albedo, f0, roughness);
    return vec4<f32>(mix(flat_light, wall_light, share) * m.finish.x, 1.0);
}
