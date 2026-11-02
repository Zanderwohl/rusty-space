// Azteced sheet metal: rectangles cut from a staggered grid by random splits, each panel one
// color and one flat normal tipped a few degrees, so panels catch the light one by one. Fully
// procedural, so nothing repeats. Each face is mapped in its own frame (u, v, n), meters, and a
// curved mesh by the face each point faces most. Decks stack along world Y, so on a side v is up,
// and windows, when there are any, are only on the sides, in rows level at the same heights all
// round; a roof, when there is one, is the whole top. Look development: crates/lc-client/src/looks/aztec.rs.

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
    @location(2) box_center: vec3<f32>,
    @location(3) box_half: vec3<f32>,
}

struct AztecUniform {
    /// `(cell_m, min_panel_m, snap_m, 0)`.
    grid: vec4<f32>,
    /// `(width_m, lean_rad, 0, 0)`: each wall of a seam's trench.
    chamfer: vec4<f32>,
    /// `(tilt_rad, brown_share, roughness, metallic)`.
    panel: vec4<f32>,
    /// Toward the star, and its turn in `w`.
    to_star: vec4<f32>,
    star_color: vec4<f32>,
    /// Direction to the planet's center, and its angular radius in `w`, radians.
    to_planet: vec4<f32>,
    planet_color: vec4<f32>,
    ambient: vec4<f32>,
    /// `(exposure, seed, unit_box, 0)`: `unit_box` is 1 where the mesh is an axis-aligned unit
    /// cube scaled by its transform, which is how a face's edges are known.
    finish: vec4<f32>,
    /// `(deck_m, pitch_m, sill_m, face_margin_m)`: no windows where `deck_m` is 0.
    deck: vec4<f32>,
    /// `(width_m, height_m, f0, roughness)`.
    pane: vec4<f32>,
    /// A lit window's radiance, and the share of its row's runs lit in `w`.
    glow: vec4<f32>,
    /// `(glow_share, glass, 0, 0)`: on a face the star lights, what of a lit window's glow is
    /// left, and the star's share every pane shows. A cheat: daylight outshines the rooms behind.
    day: vec4<f32>,
    /// `(depth_m, 0, 0, 0)`: how deep the room behind each window is.
    room: vec4<f32>,
    /// `(hex_m, seam_m, rim_m, 0)`: the top a roof of glass hexes this far across their flats, in a
    /// frame this wide, chamfered this wide where it meets the sides; no roof where `hex_m` is 0.
    roof: vec4<f32>,
    /// The air under the roof's color by day, and the share of the star it scatters back up in `w`.
    sky: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: AztecUniform;

const PI: f32 = 3.14159265;
const SPLITS: u32 = 6u;
const DESK_TOP_M: f32 = 0.75;
const FOREST_DEPTH_M: f32 = 25.0;
const CROWN_M: f32 = 5.0;
const GROVE_M: f32 = 40.0;
/// How far through the dome's air the forest fades to 1/e.
const AIR_M: f32 = 160.0;
const DESK_FROM_M: f32 = 1.2;
const DESK_DEPTH_M: f32 = 0.8;

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
    // An axis-aligned unit cube, where `unit_box` says so: the transform's scale is the box.
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

/// The axis `normal` is nearest, signed: a curved surface is mapped by the box face it faces most,
/// as a cube map would.
fn dominant(normal: vec3<f32>) -> vec3<f32> {
    let a = abs(normal);
    if (a.x >= a.y && a.x >= a.z) { return vec3<f32>(sign(normal.x), 0.0, 0.0); }
    if (a.y >= a.z) { return vec3<f32>(0.0, sign(normal.y), 0.0); }
    return vec3<f32>(0.0, 0.0, sign(normal.z));
}

/// The face's u and v laid into the surface's tangent plane, and the surface's own normal: what a
/// normal in the face's frame is turned by. The face's own frame on a flat face.
fn tangent_frame(f: Face, surface: vec3<f32>) -> mat3x3<f32> {
    let t = normalize(f.u - surface * dot(f.u, surface));
    return mat3x3<f32>(t, cross(surface, t), surface);
}

fn face(normal: vec3<f32>) -> Face {
    let n = dominant(normal);
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9);
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

    let star = turned(m.to_star);
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

struct Window {
    cover: f32,
    lit: bool,
    /// Which column and deck, which are its room's.
    col: f32,
    deck: f32,
}

/// Whether the window at `deck`, `col` is lit: mostly in runs along its row, at two lengths so
/// runs vary, with a few dark gaps in them and a few lone windows lit outside.
fn window_lit(face_id: u32, deck: i32, col: i32) -> bool {
    let row = mix_hash(mix_hash(face_id, u32(m.finish.y)), u32(deck));
    let share = m.glow.w;
    let short = i32(floor((f32(col) + 97.0 * unit(pcg(row + 1u))) / 5.0));
    let long = i32(floor((f32(col) + 97.0 * unit(pcg(row + 2u))) / 11.0));
    let run = unit(mix_hash(row + 3u, u32(short))) < 0.6 * share
        || unit(mix_hash(row + 4u, u32(long))) < 0.4 * share;
    let own = unit(mix_hash(row + 5u, u32(col)));
    return select((own < 0.06), (own > 0.12), run);
}

/// The window over `p`, if one fits wholly inside its panel clear of the chamfers and inside its
/// face clear of the margin; its cover is what of the pixel it fills.
fn window(p: vec2<f32>, cell: Panel, f: Face, surface: vec3<f32>, face_lo: vec2<f32>, face_hi: vec2<f32>, pixel: vec2<f32>) -> Window {
    // Glazing leans no more than 30 degrees off the vertical.
    if (m.deck.x <= 0.0 || abs(f.n.y) > 0.9 || abs(surface.y) > 0.5) {
        return Window(0.0, false, 0.0, 0.0);
    }
    let deck = floor(p.y / m.deck.x);
    let col = floor(p.x / m.deck.y);
    let half = 0.5 * m.pane.xy;
    let center = vec2<f32>((col + 0.5) * m.deck.y, deck * m.deck.x + m.deck.z + half.y);
    let clear = m.chamfer.x + 0.1;
    var lo = cell.lo + clear;
    var hi = cell.hi - clear;
    if (m.finish.z > 0.5) {
        lo = max(lo, face_lo + m.deck.w);
        hi = min(hi, face_hi - m.deck.w);
    }
    if (any(center - half < lo) || any(center + half > hi)) {
        return Window(0.0, false, 0.0, 0.0);
    }
    let inside = half - abs(p - center);
    let cover = clamp(inside / pixel + 0.5, vec2<f32>(0.0), min(vec2<f32>(1.0), 2.0 * half / pixel));
    return Window(cover.x * cover.y, window_lit(f.id, i32(deck), i32(col)), col, deck);
}

/// What the eye sees through the glass at `p`, as a share of a lit window's glow: a room the
/// window's column wide, its deck high and `room_m` deep, traced from the glass, with no geometry
/// behind the face at all. Interior mapping.
fn room(p: vec2<f32>, e: vec3<f32>, f: Face, pane: Window) -> vec3<f32> {
    // x and y along the face, z into it.
    let d = vec3<f32>(-dot(e, f.u), -dot(e, f.v), max(dot(e, f.n), 1.0e-3));
    let lo = vec2<f32>(pane.col * m.deck.y, pane.deck * m.deck.x);
    let hi = lo + m.deck.yx;
    let toward = select(lo, hi, d.xy > vec2<f32>(0.0));
    // Signed as `toward` was chosen, or a ray square to an axis would meet a wall behind the glass.
    let tiny = select(vec2<f32>(-1.0e-6), vec2<f32>(1.0e-6), d.xy > vec2<f32>(0.0));
    let safe = select(d.xy, tiny, abs(d.xy) < vec2<f32>(1.0e-6));
    let t_side = (toward - p) / safe;
    let t_back = m.room.x / d.z;
    let t = min(min(t_side.x, t_side.y), t_back);
    let hit = vec3<f32>(p + d.xy * t, d.z * t);

    let h = mix_hash(mix_hash(f.id, u32(i32(pane.deck))), u32(i32(pane.col)));
    let tint = mix(vec3<f32>(0.82, 0.88, 1.0), vec3<f32>(1.0, 0.9, 0.76), unit(pcg(h)));

    // A desk the room's width, its front `DESK_FROM_M` back from the glass: the ray meets its front
    // if it is below the top there, or its top if it comes down onto it.
    let desk_y = lo.y + DESK_TOP_M;
    let t_front = DESK_FROM_M / d.z;
    if (t_front < t && p.y + d.y * t_front < desk_y) {
        return tint * 0.08;
    }
    if (d.y < 0.0) {
        let t_top = (desk_y - p.y) / d.y;
        let z_top = d.z * t_top;
        if (t_top < t && z_top < DESK_FROM_M + DESK_DEPTH_M) {
            return vec3<f32>(0.6);
        }
    }

    if (t == t_back) {
        return tint * 0.5;
    }
    if (t == t_side.x) {
        return tint * mix(0.22, 0.32, unit(pcg(h + 1u)));
    }
    if (d.y > 0.0) {
        // A light panel down the middle of the ceiling.
        let across = abs(hit.x - 0.5 * (lo.x + hi.x));
        let lamp = across < 0.35 && abs(hit.z - 0.5 * m.room.x) < 0.6;
        return select(vec3<f32>(0.8), vec3<f32>(3.0), lamp);
    }
    // The floor, darker away from the light.
    return tint * 0.15 * (1.0 - 0.5 * clamp(abs(hit.z - 0.5 * m.room.x) / m.room.x, 0.0, 1.0));
}

/// Coated glass in the panel's own tilt, over `inside`, the radiance of the room behind it.
fn glass(n: vec3<f32>, e: vec3<f32>, face_n: vec3<f32>, inside: vec3<f32>) -> vec3<f32> {
    let f0 = vec3<f32>(m.pane.z);
    let roughness = m.pane.w;
    let ne = max(dot(n, e), 0.0);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - ne, 5.0);
    let star = turned(m.to_star);
    let daylight = smoothstep(-0.05, 0.25, dot(star, face_n));
    var radiance = (1.0 - fresnel) * inside * mix(1.0, m.day.x, daylight) + m.star_color.rgb * m.day.y * daylight;
    if (dot(star, face_n) > 0.0) {
        radiance += m.star_color.rgb * specular(n, star, e, f0, roughness);
    }
    radiance += m.planet_color.rgb * fresnel * planet_seen(reflect(-e, n), roughness);
    return radiance;
}

/// The offset from `p` to the center of its hex, flats `w` apart across x, and meters in from the
/// hex's edge.
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

fn value_noise(x: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(x));
    let f = fract(x);
    let u = f * f * (3.0 - 2.0 * f);
    let h00 = unit(mix_hash(u32(i.x), u32(i.y)));
    let h10 = unit(mix_hash(u32(i.x + 1), u32(i.y)));
    let h01 = unit(mix_hash(u32(i.x), u32(i.y + 1)));
    let h11 = unit(mix_hash(u32(i.x + 1), u32(i.y + 1)));
    return mix(mix(h00, h10, u.x), mix(h01, h11, u.x), u.y);
}

/// The forest at `q`, meters on its floor, as albedo times how its crowns face `star` (in the
/// face's frame): crowns `CROWN_M` apart, each a rounded dome of its own height and green drawn from
/// its grove's, the tallest at a point hiding the rest, and shadow on the floor between them.
fn canopy(q: vec2<f32>, star: vec3<f32>) -> vec3<f32> {
    let c = q / CROWN_M;
    let i = floor(c);
    let fr = c - i;
    var top = 0.0;
    var best = 9.0;
    var off = vec2<f32>(0.0);
    var tree = 0u;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let h = mix_hash(u32(i32(i.x) + x), u32(i32(i.y) + y) ^ 0x7f4a7c15u);
            let at = vec2<f32>(f32(x), f32(y)) + vec2<f32>(unit(pcg(h)), unit(pcg(h + 1u)));
            let radius = mix(0.55, 0.85, unit(pcg(h + 2u)));
            let o = (fr - at) / radius;
            let d = length(o);
            let height = mix(0.6, 1.0, unit(pcg(h + 5u))) + radius * sqrt(max(1.0 - d * d, 0.0));
            if (d < 1.0 && height > top) {
                top = height;
                best = d;
                off = o;
                tree = h;
            }
        }
    }
    var greens = array<vec3<f32>, 6>(
        vec3<f32>(0.03, 0.15, 0.02),
        vec3<f32>(0.08, 0.30, 0.03),
        vec3<f32>(0.20, 0.42, 0.03),
        vec3<f32>(0.03, 0.22, 0.10),
        vec3<f32>(0.15, 0.27, 0.05),
        vec3<f32>(0.32, 0.50, 0.06),
    );
    // A grove leans toward a couple of neighboring greens, and each tree picks among them.
    let grove = value_noise(q / GROVE_M) * 4.0;
    let pick = u32(clamp(grove + 2.0 * unit(pcg(tree + 3u)), 0.0, 5.0));
    let leaves = mix(0.75, 1.2, value_noise(q * 1.7));
    let green = greens[pick] * leaves * mix(0.85, 1.15, unit(pcg(tree + 4u)));
    if (best >= 1.0) {
        return green * 0.12;
    }
    let dome = normalize(vec3<f32>(off, sqrt(1.0 - best * best)));
    return green * (0.2 + max(dot(dome, star), 0.0));
}

/// What a roof pane shows: no light of its own, only the star's and the planet's let in through the
/// glass. A forest `FOREST_DEPTH_M` down is seen where the eye's ray meets it, so it moves against
/// the frame as the eye does, through air that scatters the light back blue by day and red as the
/// star sets over the roof; the longer the ray's path, the more air hides the forest, so a glancing
/// look sees sky only. Nothing by night.
fn sky_below(p: vec2<f32>, f: Face, surface: vec3<f32>, e: vec3<f32>, edge: f32) -> vec3<f32> {
    let star = turned(m.to_star);
    let over = dot(star, surface);
    let day = smoothstep(-0.08, 0.25, over);
    let high = smoothstep(0.0, 0.45, over);
    let tint = mix(vec3<f32>(1.0, 0.42, 0.16), m.sky.rgb, high);
    let rayleigh = 0.75 * (1.0 + pow(dot(-star, e), 2.0));
    let air = m.star_color.rgb * m.sky.w * day * mix(vec3<f32>(1.0), tint, 0.7) * rayleigh;

    let d = vec3<f32>(-dot(e, f.u), -dot(e, f.v), max(dot(e, f.n), 0.02));
    let path = FOREST_DEPTH_M / d.z;
    let through = exp(-path / AIR_M);
    let sunlight = m.star_color.rgb * day * mix(vec3<f32>(1.0, 0.6, 0.35), vec3<f32>(1.0), high);
    let star_face = vec3<f32>(dot(star, f.u), dot(star, f.v), dot(star, f.n));
    let forest = canopy(p + d.xy * path, star_face) * sunlight / PI;

    var below = forest * through + air * (1.0 - through);
    // The frame shades what is just inside it.
    below *= mix(0.55, 1.0, smoothstep(0.0, 2.0, edge));
    let planet = m.planet_color.rgb * m.sky.rgb * max(dot(normalize(m.to_planet.xyz), surface), 0.0) * 0.3;
    return below + planet;
}

struct Rim {
    share: f32,
    normal: vec3<f32>,
}

/// The chamfer where the roof meets the sides, `roof.z` across: each side of the line leans 45°
/// toward it, so at a box's edge the two strips are one bevel, and on a curved hull a groove.
fn rim(in: VertexOutput, f: Face, surface: vec3<f32>, p: vec2<f32>) -> Rim {
    let up = vec3<f32>(0.0, 1.0, 0.0);
    var dist = 1.0e6;
    var toward = up;
    if (m.finish.z > 0.5) {
        if (f.n.y > 0.5) {
            let at = vec2<f32>(dot(in.box_center, f.u), dot(in.box_center, f.v));
            let half = vec2<f32>(dot(abs(f.u), in.box_half), dot(abs(f.v), in.box_half));
            let to_edge = vec4<f32>(p - (at - half), (at + half) - p);
            dist = min(min(to_edge.x, to_edge.y), min(to_edge.z, to_edge.w));
            var out = vec2<f32>(0.0, 1.0);
            if (dist == to_edge.x) { out = vec2<f32>(-1.0, 0.0); }
            else if (dist == to_edge.y) { out = vec2<f32>(0.0, -1.0); }
            else if (dist == to_edge.z) { out = vec2<f32>(1.0, 0.0); }
            toward = f.u * out.x + f.v * out.y;
        } else if (abs(f.n.y) < 0.5) {
            dist = in.box_center.y + in.box_half.y - in.world_position.y;
        }
    } else {
        // How far the roof's face is from winning, over how fast that changes a meter.
        let lead = surface.y - max(abs(surface.x), abs(surface.z));
        let per_pixel = length(vec2<f32>(dpdx(lead), dpdy(lead)));
        let pixel_m = max(length(fwidth(in.world_position)) * 0.7071, 1.0e-4);
        dist = abs(lead) * pixel_m / max(per_pixel, 1.0e-6);
        let side = vec3<f32>(surface.x, 0.0, surface.z) + vec3<f32>(1.0e-6, 0.0, 0.0);
        let along = select(up, normalize(side), lead > 0.0);
        toward = normalize(along - surface * dot(along, surface));
    }
    let half_width = 0.5 * m.roof.z;
    let pixel = max(length(fwidth(p)), 1.0e-4);
    let share = (1.0 - smoothstep(half_width - pixel, half_width + pixel, dist)) * clamp(half_width / pixel, 0.0, 1.0);
    return Rim(share, normalize(surface * 0.7071 + toward * 0.7071));
}

/// The roof's metal, as its frame and its rim are.
fn roof_metal(n: vec3<f32>, e: vec3<f32>, surface: vec3<f32>) -> vec3<f32> {
    let base = vec3<f32>(0.4);
    let metallic = m.panel.w;
    return lit(n, e, surface, base * (1.0 - metallic), mix(vec3<f32>(0.04), base, metallic), m.panel.z);
}

/// The top's roof: clear glass hexes in a metal frame, the glass over [`sky_below`].
fn roof(p: vec2<f32>, f: Face, surface: vec3<f32>, e: vec3<f32>) -> vec3<f32> {
    let cell = hex(p, m.roof.x);
    let frame = roof_metal(surface, e, surface);

    let f0 = vec3<f32>(0.04);
    let roughness = 0.05;
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - max(dot(surface, e), 0.0), 5.0);
    let half_seam = 0.5 * m.roof.y;
    var pane = (1.0 - fresnel) * sky_below(p, f, surface, e, cell.z - half_seam);
    let star = turned(m.to_star);
    if (dot(star, surface) > 0.0) {
        pane += m.star_color.rgb * specular(surface, star, e, f0, roughness);
    }
    pane += m.planet_color.rgb * fresnel * planet_seen(reflect(-e, surface), roughness);

    let pixel = max(length(fwidth(p)), 1.0e-4);
    let glass = smoothstep(half_seam - pixel, half_seam + pixel, cell.z);
    return mix(frame, pane, glass);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let surface = normalize(in.world_normal);
    let f = face(surface);
    let frame = tangent_frame(f, surface);
    let p = vec2<f32>(dot(in.world_position, f.u), dot(in.world_position, f.v));
    let e = normalize(view.world_position - in.world_position);
    var edged = Rim(0.0, surface);
    if (m.roof.x > 0.0) {
        edged = rim(in, f, surface, p);
    }
    let bevel = roof_metal(edged.normal, e, surface);
    if (m.roof.x > 0.0 && f.n.y > 0.5) {
        return vec4<f32>(mix(roof(p, f, surface, e), bevel, edged.share) * m.finish.x, 1.0);
    }
    let cell = panel(p, f.id);

    let n = normalize(frame * tipped(cell.id));
    let base = paint(cell.id);
    let roughness = m.panel.z;
    let metallic = m.panel.w;
    let albedo = base * (1.0 - metallic);
    let f0 = mix(vec3<f32>(0.04), base, metallic);

    // A chamfer slopes down into the seam from each side, so the two walls of the trench face
    // each other. Its normal leans toward the nearest edge, which miters the corners.
    let to_edge = vec4<f32>(p.x - cell.lo.x, cell.hi.x - p.x, p.y - cell.lo.y, cell.hi.y - p.y);
    let edge = min(min(to_edge.x, to_edge.y), min(to_edge.z, to_edge.w));
    var outward = vec2<f32>(0.0, 1.0);
    if (edge == to_edge.x) { outward = vec2<f32>(-1.0, 0.0); }
    else if (edge == to_edge.y) { outward = vec2<f32>(1.0, 0.0); }
    else if (edge == to_edge.z) { outward = vec2<f32>(0.0, -1.0); }
    let lean = m.chamfer.y;
    let wall = normalize(n * cos(lean) + (frame[0] * outward.x + frame[1] * outward.y) * sin(lean));

    // Where a pixel spans the wall, its share of the pixel stands in for it.
    let pixel = max(length(fwidth(p)), 1.0e-4);
    let width = m.chamfer.x;
    let share = (1.0 - smoothstep(width - pixel, width + pixel, edge)) * clamp(width / pixel, 0.0, 1.0);

    let flat_light = lit(n, e, surface, albedo, f0, roughness);
    let wall_light = lit(wall, e, surface, albedo, f0, roughness);
    let plate = mix(flat_light, wall_light, share);
    let box_at = vec2<f32>(dot(in.box_center, f.u), dot(in.box_center, f.v));
    let box_half = vec2<f32>(dot(abs(f.u), in.box_half), dot(abs(f.v), in.box_half));
    let pane = window(p, cell, f, surface, box_at - box_half, box_at + box_half, max(fwidth(p), vec2<f32>(1.0e-4)));
    var radiance = plate;
    if (pane.cover > 0.0) {
        let lights = select(m.ambient.rgb * 0.2, m.glow.rgb, pane.lit);
        radiance = mix(plate, glass(n, e, surface, lights * room(p, e, f, pane)), pane.cover);
    }
    return vec4<f32>(mix(radiance, bevel, edged.share) * m.finish.x, 1.0);
}
