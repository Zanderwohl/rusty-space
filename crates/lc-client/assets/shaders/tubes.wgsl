// Launch tubes in a hive. Hexagonal cells, a groove between them, each with a tube at its middle
// ringed in red and white danger tape and closed by a six-bladed iris, shut, half open or open. An
// open tube is traced: the view ray meets its wall, ringed and lit by guide lights running down it,
// or a drone's nose, or the empty floor. A margin two apothems wide around each face is bare plate,
// so no cell is cut by an edge. Each face is mapped in its own frame (u, v, n), meters. Look
// development: crates/lc-client/src/looks/tubes.rs.

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
    @location(2) box_center: vec3<f32>,
    @location(3) box_half: vec3<f32>,
}

struct TubesUniform {
    /// `(hex_m, bore_radius_m, tape_m, 0)`: a cell across its flats, the tube's radius, and the
    /// danger tape's width around its mouth.
    cell: vec4<f32>,
    /// `(depth_m, drone_m, open, loaded)`: the tube's depth, how far down a drone's nose is, the
    /// share of tubes open, and the share of open tubes holding a drone.
    tube: vec4<f32>,
    /// A guide light's radiance, linear, and their spacing down the wall in `w`.
    guide: vec4<f32>,
    /// The plate's paint, linear.
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

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: TubesUniform;

const PI: f32 = 3.14159265;
const GROOVE_M: f32 = 0.12;
const GROOVE_LEAN: f32 = 1.0;
/// The tape's rounded lip at the mouth, and a stripe's width along it.
const LIP_M: f32 = 0.12;
const STRIPE_M: f32 = 0.3;
const TAPE_RED: vec3<f32> = vec3<f32>(0.6, 0.03, 0.02);
const TAPE_WHITE: vec3<f32> = vec3<f32>(0.7, 0.7, 0.68);
const BLADES: f32 = 6.0;
/// How far each blade sweeps around as it runs from the rim to the aperture, in turns.
const BLADE_SWEEP: f32 = 0.35;
const BLADE_LEAN: f32 = 0.3;
const RING_PITCH_M: f32 = 1.5;
const GUIDE_M: f32 = 0.08;
/// How far a drone's nose is across, as a share of the bore.
const NOSE_SHARE: f32 = 0.75;
const EYE: vec3<f32> = vec3<f32>(1.0, 0.08, 0.04) * 2.0;

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
    // An axis-aligned unit cube: the transform's scale is the box.
    out.box_center = world_from_local[3].xyz;
    out.box_half = 0.5 * vec3<f32>(length(world_from_local[0].xyz), length(world_from_local[1].xyz), length(world_from_local[2].xyz));
    return out;
}

struct Face {
    u: vec3<f32>,
    v: vec3<f32>,
    n: vec3<f32>,
    id: u32,
}

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

fn tangent_frame(f: Face, surface: vec3<f32>) -> mat3x3<f32> {
    let t = normalize(f.u - surface * dot(f.u, surface));
    return mat3x3<f32>(t, cross(surface, t), surface);
}

/// `p` less its hex's center, flats `w` apart across u, and meters in from the hex's edge.
fn hex(p: vec2<f32>, w: f32) -> vec3<f32> {
    let r = vec2<f32>(1.0, 1.7320508) * w;
    let h = 0.5 * r;
    let a = p - r * floor(p / r) - h;
    let q = p - h;
    let b = q - r * floor(q / r) - h;
    let off = select(b, a, dot(a, a) < dot(b, b));
    let o = abs(off);
    return vec3<f32>(off, 0.5 * w - max(o.x, dot(o, vec2<f32>(0.5, 0.8660254))));
}

/// Out from a hex's middle square to its nearest edge.
fn edge_out(off: vec2<f32>) -> vec2<f32> {
    let o = abs(off);
    if (o.x >= dot(o, vec2<f32>(0.5, 0.8660254))) {
        return vec2<f32>(sign(off.x), 0.0);
    }
    return vec2<f32>(0.5 * sign(off.x), 0.8660254 * sign(off.y));
}

fn ggx(n: vec3<f32>, h: vec3<f32>, roughness: f32) -> f32 {
    let a2 = roughness * roughness * roughness * roughness;
    let nh = max(dot(n, h), 0.0);
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

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

/// A lean that fades to nothing where its feature is narrower than a pixel.
fn steady(lean: f32, half: f32, pixel: f32) -> f32 {
    return lean * clamp(half / pixel - 0.5, 0.0, 1.0);
}

/// A light `radius` across at `dist`, its energy kept as it spreads to a pixel's width.
fn spot(dist: f32, radius: f32, pixel: f32) -> f32 {
    let r = max(radius, pixel);
    return (1.0 - smoothstep(0.6 * r, r, dist)) * (radius * radius) / (r * r);
}

/// Where the ray from `q` along `d` meets a circle of `radius` about the axis, leaving it from
/// inside (`exit`) or entering it from outside. A huge `t` for a ray along the axis.
fn circle(q: vec2<f32>, d: vec2<f32>, radius: f32, exit: bool) -> f32 {
    let a = dot(d, d);
    let b = 2.0 * dot(q, d);
    let c = dot(q, q) - radius * radius;
    let disc = b * b - 4.0 * a * c;
    if (a < 1.0e-8 || disc < 0.0) {
        return 1.0e9;
    }
    let root = sqrt(disc);
    return select((-b - root) / (2.0 * a), (-b + root) / (2.0 * a), exit);
}

/// What the eye sees down an open tube from `q`, meters from its axis on the mouth, looking along
/// `d`, which is in the face's frame with z into it. `daylight` is what spills in at the mouth.
fn down_tube(q: vec2<f32>, d: vec3<f32>, loaded: bool, daylight: vec3<f32>, pixel: f32) -> vec3<f32> {
    let r = m.cell.y;
    let depth = m.tube.x;
    let guide = m.guide.rgb;
    var t = min(circle(q, d.xy, r, true), depth / d.z);
    var what = 0;
    let nose_r = NOSE_SHARE * r;
    let nose_c = vec3<f32>(0.0, 0.0, m.tube.y + nose_r);
    if (loaded) {
        // The nose is a hemisphere, its tip `drone_m` down; the body behind it a cylinder.
        let o = vec3<f32>(q, 0.0) - nose_c;
        let b = dot(o, d);
        let disc = b * b - dot(d, d) * (dot(o, o) - nose_r * nose_r);
        if (disc > 0.0) {
            let tn = (-b - sqrt(disc)) / dot(d, d);
            if (tn > 0.0 && tn < t && d.z * tn <= nose_c.z) {
                t = tn;
                what = 1;
            }
        }
        let tb = circle(q, d.xy, nose_r, false);
        if (tb > 0.0 && tb < t && d.z * tb > nose_c.z) {
            t = tb;
            what = 2;
        }
    }
    let hit = vec3<f32>(q + d.xy * t, d.z * t);
    // Light from the mouth falls off with depth, about as a tube's width.
    let spill = daylight * exp(-hit.z / r);
    let glow_fill = guide * 0.012;
    if (what == 1) {
        let n = normalize(hit - nose_c);
        let base = vec3<f32>(0.75, 0.7, 0.6);
        // Lit by the guide lights around it as much as by the mouth, and its sensor eye awake.
        let eye = spot(length(hit.xy), 0.2 * nose_r, pixel);
        return base * (spill * max(-n.z, 0.0) + 6.0 * glow_fill) + EYE * eye;
    }
    if (what == 2) {
        return vec3<f32>(0.12) * (spill * 0.3 + glow_fill);
    }
    if (hit.z >= depth - 1.0e-3) {
        // The empty floor, a landing ring marked on it.
        let ring = 1.0 - smoothstep(0.04, 0.08, abs(length(hit.xy) - 0.6 * r));
        return vec3<f32>(0.06) * (spill + glow_fill) + 0.3 * guide * ring * exp(-depth / (4.0 * r));
    }
    // The wall: ringed, with guide lights in six columns down it.
    let banded = select(1.0, 0.4, fract(hit.z / RING_PITCH_M) < 0.08);
    var c = vec3<f32>(0.22) * banded * (spill + glow_fill);
    let phi = atan2(hit.y, hit.x);
    let column = PI / 3.0;
    let k = round((phi - 0.5 * column) / column);
    let arc = (phi - (k + 0.5) * column) * r;
    let pitch = m.guide.w;
    let along = hit.z - (floor(hit.z / pitch) + 0.5) * pitch;
    let dist = length(vec2<f32>(arc, along));
    c += guide * (spot(dist, GUIDE_M, pixel) + 0.025 * exp(-dist / 0.25));
    return c;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let surface = normalize(in.world_normal);
    let f = face(surface);
    let frame = tangent_frame(f, surface);
    let p = vec2<f32>(dot(in.world_position, f.u), dot(in.world_position, f.v));
    let e = normalize(view.world_position - in.world_position);
    let pixel = max(length(fwidth(p)), 1.0e-4);
    let star = turned(m.to_star);

    let w = m.cell.x;
    let h = hex(p, w);
    let off = h.xy;
    let center = p - off;
    let box_at = vec2<f32>(dot(in.box_center, f.u), dot(in.box_center, f.v));
    let box_half = vec2<f32>(dot(abs(f.u), in.box_half), dot(abs(f.v), in.box_half));
    // Two apothems, a cell's width, between a kept cell's middle and the edge.
    let kept = all(abs(center - box_at) <= box_half - vec2<f32>(w));
    if (!kept) {
        return vec4<f32>(lit(surface, e, surface, 0.9 * m.paint.rgb) * m.finish.x, 1.0);
    }

    let seed = mix_hash(u32(m.finish.y), f.id);
    let id = mix_hash(mix_hash(seed, u32(i32(round(center.x * 4.0)))), u32(i32(round(center.y * 4.0))));
    let base = m.paint.rgb * mix(0.92, 1.08, unit(pcg(id + 3u)));
    var radiance = lit(surface, e, surface, base);

    // The groove between cells: its walls lean toward the edge.
    let groove_half = 0.5 * GROOVE_M;
    let groove_share = (1.0 - smoothstep(groove_half - pixel, groove_half + pixel, h.z)) * clamp(GROOVE_M / pixel, 0.0, 1.0);
    let toward_edge = edge_out(off) * steady(GROOVE_LEAN, groove_half, pixel);
    let groove_n = normalize(frame * vec3<f32>(toward_edge, 1.0));
    radiance = mix(radiance, lit(groove_n, e, surface, 0.35 * base), groove_share);

    let r = m.cell.y;
    let tape = m.cell.z;
    let d = length(off);
    if (d < r + tape + pixel) {
        let radial = select(vec2<f32>(1.0, 0.0), off / d, d > 1.0e-4);
        let phi = atan2(off.y, off.x);

        // Diagonal stripes around the mouth, and a rounded lip at its inner edge.
        let s = (phi * (r + 0.5 * tape) + (d - r)) / STRIPE_M;
        let stripe_aa = 2.0 * pixel / STRIPE_M;
        let white = smoothstep(0.5 - stripe_aa, 0.5 + stripe_aa, abs(fract(0.5 * s) * 2.0 - 1.0));
        let tape_color = mix(TAPE_RED, TAPE_WHITE, white);
        let lip = clamp(1.0 - (d - r) / LIP_M, 0.0, 1.0);
        let tape_n = normalize(frame * vec3<f32>(-radial * lip * steady(1.2, 0.5 * LIP_M, pixel), 1.0));
        let tape_light = lit(tape_n, e, surface, tape_color);
        radiance = mix(radiance, tape_light, 1.0 - smoothstep(r + tape - pixel, r + tape + pixel, d));

        if (d < r + pixel) {
            let state = unit(pcg(id + 5u));
            let open = m.tube.z;
            let aperture = select(select(0.0, 0.5 * r, state < open + 0.12), 0.97 * r, state < open);
            // Each blade sweeps around as it runs in, and leans so that each overlaps the next.
            let b = BLADES * fract(phi / (2.0 * PI) + BLADE_SWEEP * (r - d) / r);
            let t_b = fract(b);
            let seam_aa = BLADES * pixel / (2.0 * PI * max(d, 0.1)) * 2.0;
            let seam = smoothstep(0.0, seam_aa + 0.02, t_b) * smoothstep(0.0, seam_aa + 0.02, 1.0 - t_b);
            let tangent = vec2<f32>(-radial.y, radial.x);
            let blade_n = normalize(frame * vec3<f32>(tangent * (t_b - 0.5) * steady(BLADE_LEAN, 0.25 * r, pixel), 1.0));
            let blades = lit(blade_n, e, surface, vec3<f32>(0.24) * mix(0.3, 1.0, seam));
            radiance = mix(radiance, blades, 1.0 - smoothstep(r - pixel, r + pixel, d));

            if (d < aperture + pixel) {
                let loaded = unit(pcg(id + 9u)) < m.tube.w;
                let into = vec3<f32>(-dot(e, f.u), -dot(e, f.v), max(dot(e, f.n), 1.0e-3));
                let daylight = m.star_color.rgb * max(dot(star, surface), 0.0) * 0.2 + m.ambient.rgb;
                let inside = down_tube(off, into, loaded, daylight, pixel);
                radiance = mix(radiance, inside, 1.0 - smoothstep(aperture - pixel, aperture + pixel, d));
            }
        }
    }
    return vec4<f32>(radiance * m.finish.x, 1.0);
}
