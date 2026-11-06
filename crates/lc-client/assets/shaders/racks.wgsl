// Server racks. On a side, servers a deck tall stacked between rounded rack posts, a V-groove
// between each server and the next, each server its own shade and tipped a little so they catch
// the light one by one. On the top and bottom, a data hall from above: rows of rack tops, each
// with a fan, between sunken aisles. Each face is mapped in its own frame (u, v, n), meters, with
// v up on a side. Look development: crates/lc-client/src/looks/racks.rs.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::{globals, view},
    view_transformations::position_world_to_clip,
}
#import "shaders/hash.wgsl"::{pcg, mix_hash, unit}

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

struct RacksUniform {
    /// `(rack_m, server_m, post_m, groove_m)`: a server's width and height, a post's width, and the
    /// groove's between servers.
    rack: vec4<f32>,
    /// `(post, groove, tilt_rad, 0)`: how far a post's faces and a groove's walls lean at most, as
    /// tangents, and how far a server may tip.
    lean: vec4<f32>,
    /// `(depth_m, aisle_m, fan_m, 0)`: from above, a row of rack tops, the aisle between rows, and
    /// a rack top's fan across.
    hall: vec4<f32>,
    /// The servers' paint, linear, and how far each strays from it in `w`.
    paint: vec4<f32>,
    /// `(roughness, metallic, 0, 0)`.
    surface: vec4<f32>,
    /// Toward the star, and its turn in `w`.
    to_star: vec4<f32>,
    star_color: vec4<f32>,
    /// Direction to the planet's center, and its angular radius in `w`, radians.
    to_planet: vec4<f32>,
    planet_color: vec4<f32>,
    ambient: vec4<f32>,
    /// `(exposure, seed, 0, 0)`.
    finish: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: RacksUniform;

const PI: f32 = 3.14159265;
/// A sunken aisle's wall, across the face.
const AISLE_WALL_M: f32 = 0.3;
/// A fan's grille: rings this far apart, leaning this far as a tangent, and its hub's share of it.
const RING_M: f32 = 0.25;
const RING_LEAN: f32 = 0.5;
const HUB_SHARE: f32 = 0.22;

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

struct Face {
    u: vec3<f32>,
    v: vec3<f32>,
    n: vec3<f32>,
    id: u32,
}

/// The axis `normal` is nearest, signed: a curved surface is mapped by the box face it faces most.
fn dominant(normal: vec3<f32>) -> vec3<f32> {
    let a = abs(normal);
    if (a.x >= a.y && a.x >= a.z) { return vec3<f32>(sign(normal.x), 0.0, 0.0); }
    if (a.y >= a.z) { return vec3<f32>(0.0, sign(normal.y), 0.0); }
    return vec3<f32>(0.0, 0.0, sign(normal.z));
}

fn face(normal: vec3<f32>) -> Face {
    let n = dominant(normal);
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    let u = normalize(cross(up, n));
    return Face(u, cross(n, u), n, u32(dot(round(n), vec3<f32>(1.0, 3.0, 9.0)) + 13.0));
}

/// The face's u and v laid into the surface's tangent plane, and the surface's own normal.
fn tangent_frame(f: Face, surface: vec3<f32>) -> mat3x3<f32> {
    let t = normalize(f.u - surface * dot(f.u, surface));
    return mat3x3<f32>(t, cross(surface, t), surface);
}

/// A normal in the face's frame, tipped by up to `tilt` radians, uniform over that disc.
fn tipped(id: u32, tilt: f32) -> vec3<f32> {
    let r = tilt * sqrt(unit(pcg(id + 7u)));
    let a = 2.0 * PI * unit(pcg(id + 11u));
    return vec3<f32>(sin(r) * cos(a), sin(r) * sin(a), cos(r));
}

/// A shade of the paint, its own for `id`.
fn shade(id: u32) -> vec3<f32> {
    return m.paint.rgb * (1.0 + m.paint.w * (2.0 * unit(pcg(id + 3u)) - 1.0));
}

/// `x` from the nearest multiple of `pitch`, in `[-pitch / 2, pitch / 2]`.
fn offset(x: f32, pitch: f32) -> f32 {
    return x - pitch * round(x / pitch);
}

/// How much of a pixel `pixel` wide at `off` from a line lies in a band `half` to each side of it.
/// Where the band is narrower than the pixel, its share of the pixel.
fn cover(off: f32, half: f32, pixel: f32) -> f32 {
    return (1.0 - smoothstep(half - pixel, half + pixel, abs(off))) * clamp(2.0 * half / pixel, 0.0, 1.0);
}

/// A lean that fades to nothing where its band is narrower than a pixel, so it averages flat
/// rather than sparkling.
fn steady(lean: f32, half: f32, pixel: f32) -> f32 {
    return lean * clamp(half / pixel - 0.5, 0.0, 1.0);
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

fn planet_seen(r: vec3<f32>, roughness: f32) -> f32 {
    let off = acos(clamp(dot(r, normalize(m.to_planet.xyz)), -1.0, 1.0));
    let blur = 0.5 * roughness * roughness + 0.01;
    return 1.0 - smoothstep(m.to_planet.w - blur, m.to_planet.w + blur, off);
}

/// What paint `base` with normal `n` sends toward the eye `e`, on a surface facing `face_n`.
fn lit(n: vec3<f32>, e: vec3<f32>, face_n: vec3<f32>, base: vec3<f32>) -> vec3<f32> {
    let roughness = m.surface.x;
    let metallic = m.surface.y;
    let albedo = base * (1.0 - metallic);
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    var radiance = m.ambient.rgb * albedo;
    let star = turned(m.to_star);
    if (dot(star, face_n) > 0.0) {
        let ndl = max(dot(n, star), 0.0);
        radiance += m.star_color.rgb * (albedo * ndl / PI + specular(n, star, e, f0, roughness));
    }
    let to_planet = normalize(m.to_planet.xyz);
    let solid = 2.0 * PI * (1.0 - cos(m.to_planet.w));
    radiance += m.planet_color.rgb * albedo * max(dot(n, to_planet), 0.0) * solid / PI;
    let ne = max(dot(n, e), 0.0);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - ne, 5.0);
    radiance += m.planet_color.rgb * fresnel * planet_seen(reflect(-e, n), roughness);
    return radiance;
}

/// A side: servers between posts, a groove between each and the next.
fn side(p: vec2<f32>, f: Face, frame: mat3x3<f32>, surface: vec3<f32>, e: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    let rack = m.rack.x;
    let server = m.rack.y;
    let seed = mix_hash(u32(m.finish.y), f.id);
    let id = mix_hash(mix_hash(seed, u32(i32(floor(p.x / rack)))), u32(i32(floor(p.y / server))));
    let base = shade(id);
    var radiance = lit(normalize(frame * tipped(id, m.lean.z)), e, surface, base);

    // Each wall faces away from the groove's floor: the one above it down, the one below it up.
    let gy = offset(p.y, server);
    let gh = 0.5 * m.rack.w;
    let groove = normalize(vec3<f32>(0.0, -sign(gy) * steady(m.lean.y, gh, pixel.y), 1.0));
    radiance = mix(radiance, lit(frame * groove, e, surface, 0.35 * base), cover(gy, gh, pixel.y));

    // Rounded and standing proud, so a post's face leans out toward its edges.
    let px = offset(p.x, rack);
    let ph = 0.5 * m.rack.z;
    let post = normalize(vec3<f32>(clamp(px / ph, -1.0, 1.0) * steady(m.lean.x, ph, pixel.x), 0.0, 1.0));
    return mix(radiance, lit(frame * post, e, surface, 1.3 * m.paint.rgb), cover(px, ph, pixel.x));
}

/// A top or bottom: rows of rack tops along u, each a fan in every rack's width, between aisles.
fn hall(p: vec2<f32>, f: Face, frame: mat3x3<f32>, surface: vec3<f32>, e: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    let rack = m.rack.x;
    let depth = m.hall.x;
    let pitch = depth + m.hall.y;
    let row = floor(p.y / pitch);
    let across = p.y - row * pitch;
    let seed = mix_hash(u32(m.finish.y), f.id);
    let id = mix_hash(mix_hash(seed, u32(i32(row))), u32(i32(floor(p.x / rack))));
    let base = shade(id);

    // A rack top, and the groove between it and the next along its row.
    let middle = vec2<f32>((floor(p.x / rack) + 0.5) * rack, row * pitch + 0.5 * depth);
    var top = lit(normalize(frame * tipped(id, m.lean.z)), e, surface, base);
    let gx = offset(p.x, rack);
    let gh = 0.5 * m.rack.w;
    let groove = normalize(vec3<f32>(-sign(gx) * steady(m.lean.y, gh, pixel.x), 0.0, 1.0));
    top = mix(top, lit(frame * groove, e, surface, 0.35 * base), cover(gx, gh, pixel.x));

    // The fan: a dark grille of rings about a hub, each ring's walls leaning in and out.
    let radius = 0.5 * m.hall.z;
    let to_fan = p - middle;
    let d = length(to_fan);
    let outward = select(vec2<f32>(0.0), to_fan / d, d > 1.0e-4);
    let fine = max(pixel.x, pixel.y);
    let ring = sin(2.0 * PI * d / RING_M) * steady(RING_LEAN, 0.25 * RING_M, fine);
    let grille = normalize(vec3<f32>(outward * ring, 1.0));
    let hub = 1.0 - smoothstep(HUB_SHARE * radius - fine, HUB_SHARE * radius + fine, d);
    let fan = mix(lit(frame * grille, e, surface, 0.3 * m.paint.rgb), lit(surface, e, surface, 0.8 * base), hub);
    top = mix(top, fan, 1.0 - smoothstep(radius - fine, radius + fine, d));

    // The aisle is sunk, its walls facing into it from the rows on either side.
    let into = across - depth;
    let aisle_w = m.hall.y;
    let wall_v = select(-1.0, 1.0, into < 0.5 * aisle_w);
    let near_wall = min(into, aisle_w - into);
    let wall_share = 1.0 - smoothstep(AISLE_WALL_M - fine, AISLE_WALL_M + fine, near_wall);
    let wall = normalize(vec3<f32>(0.0, wall_v, 1.0));
    let floor_light = mix(lit(surface, e, surface, 0.4 * m.paint.rgb), lit(frame * wall, e, surface, 0.6 * m.paint.rgb), wall_share);
    let in_aisle = smoothstep(-pixel.y, pixel.y, into) * (1.0 - smoothstep(aisle_w - pixel.y, aisle_w + pixel.y, into));
    return mix(top, floor_light, in_aisle);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let surface = normalize(in.world_normal);
    let f = face(surface);
    let frame = tangent_frame(f, surface);
    let p = vec2<f32>(dot(in.world_position, f.u), dot(in.world_position, f.v));
    let e = normalize(view.world_position - in.world_position);
    let pixel = max(fwidth(p), vec2<f32>(1.0e-4));
    var radiance: vec3<f32>;
    if (abs(f.n.y) > 0.5) {
        radiance = hall(p, f, frame, surface, e, pixel);
    } else {
        radiance = side(p, f, frame, surface, e, pixel);
    }
    return vec4<f32>(radiance * m.finish.x, 1.0);
}
