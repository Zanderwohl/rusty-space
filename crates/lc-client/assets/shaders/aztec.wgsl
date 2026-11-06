// Azteced sheet metal: rectangles cut from a staggered grid by random splits, each panel one
// color and one flat normal tipped a few degrees, so panels catch the light one by one. Fully
// procedural, so nothing repeats. Laid out in the part's own charts (crate::charts in lc-client),
// a whole number of grid cells around a band or ring. Decks stack up a chart's v, and windows, when
// there are any, are only on a part's sides, clear of each chart's edges; a roof, when there is
// one, is the part's end that faces up, the well under its glass walled by that end's edge. Look
// development: crates/lc-client/src/looks/aztec.rs.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::{globals, view},
    view_transformations::position_world_to_clip,
}
#import "shaders/charts.wgsl"::{chart_view}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // em_render::hull_material::ATTRIBUTE_HULL_CHART and ATTRIBUTE_HULL_CHART_BOUNDS.
    @location(2) chart: vec4<f32>,
    @location(3) bounds: vec4<f32>,
    @location(4) tangent: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) chart: vec4<f32>,
    @location(3) @interpolate(flat) bounds: vec4<f32>,
    @location(4) world_tangent: vec3<f32>,
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
    /// `(exposure, seed, 0, 0)`.
    finish: vec4<f32>,
    /// `(deck_m, pitch_m, sill_m, chart_margin_m)`: no windows where `deck_m` is 0.
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
    /// `(most_m, share, 0, 0)`: the forest lies `share` of the roof's width down, and no deeper
    /// than `most_m`.
    forest: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: AztecUniform;

const RECT: u32 = 0u;
const DISC: u32 = 1u;
const BAND: u32 = 2u;
const RING: u32 = 3u;
const DOME: u32 = 4u;

/// The fragment's chart: its kind, its bounds, and how far its u and v run around before they
/// repeat, fitted to a whole number of grid cells; 0 where they do not.
var<private> chart_kind: u32;
var<private> chart_bounds: vec4<f32>;
var<private> around: vec2<f32>;

const PI: f32 = 3.14159265;
const SPLITS: u32 = 6u;
const DESK_TOP_M: f32 = 0.75;
const CROWN_M: f32 = 5.0;
const GROVE_M: f32 = 40.0;
/// How far down the stratus lies, as a share of the forest's depth.
const STRATUS_SHARE: f32 = 0.4;
const WALL_STEPS: i32 = 32;
const WALL_ALBEDO: f32 = 0.25;
const STRATUS_SCALE_M: vec2<f32> = vec2<f32>(35.0, 12.0);
/// What of the forest the thickest stratus hides.
const STRATUS_ALPHA: f32 = 0.6;
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
    out.chart = vertex.chart;
    out.bounds = vertex.bounds;
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex.tangent, vertex.instance_index).xyz;
    return out;
}

/// `x` modulo `n`, never negative.
fn wrap(x: i32, n: i32) -> i32 {
    return ((x % n) + n) % n;
}

/// The index `i` of a lattice `pitch` apart, taken around `period` where there is one, so the
/// lattice's cells either side of a seam are one.
fn around_index(i: f32, pitch: f32, period: f32) -> u32 {
    if (period > 0.0) {
        return u32(wrap(i32(i), i32(round(period / pitch))));
    }
    return u32(i32(i));
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

/// Grid snapped coordinates are exact, so a panel's bounds name it whichever pixel asks, and on
/// either side of a seam.
fn snapped_id(x: f32, period: f32) -> u32 {
    return around_index(round(x / m.grid.z), m.grid.z, period);
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
    let row_id = around_index(row, cell, around.y);
    let shift = round(unit(mix_hash(seed, row_id)) * cell / snap) * snap;
    let col = floor((p.x - shift) / cell);
    var lo = vec2<f32>(col * cell + shift, row * cell);
    var hi = lo + vec2<f32>(cell);
    var id = mix_hash(mix_hash(seed, row_id), around_index(col, cell, around.x));
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
        let a = snapped_id(lo.x, around.x) ^ (snapped_id(lo.y, around.y) << 16u);
        let b = snapped_id(hi.x, around.x) ^ (snapped_id(hi.y, around.y) << 16u);
        id = mix_hash(mix_hash(a, b), seed);
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
fn window_lit(face_id: u32, deck_at: f32, col_at: f32) -> bool {
    let deck = around_index(deck_at, m.deck.x, around.y);
    let col = i32(around_index(col_at, m.deck.y, around.x));
    let row = mix_hash(mix_hash(face_id, u32(m.finish.y)), deck);
    let share = m.glow.w;
    let short = i32(floor((f32(col) + 97.0 * unit(pcg(row + 1u))) / 5.0));
    let long = i32(floor((f32(col) + 97.0 * unit(pcg(row + 2u))) / 11.0));
    let run = unit(mix_hash(row + 3u, u32(short))) < 0.6 * share
        || unit(mix_hash(row + 4u, u32(long))) < 0.4 * share;
    let own = unit(mix_hash(row + 5u, u32(col)));
    return select((own < 0.06), (own > 0.12), run);
}

/// The window over `p`, if one fits wholly inside its panel clear of the chamfers and inside its
/// chart clear of the margin; its cover is what of the pixel it fills. On a part's sides only.
fn window(p: vec2<f32>, cell: Panel, face_id: u32, end: bool, surface: vec3<f32>, pixel: vec2<f32>) -> Window {
    // Glazing leans no more than 30 degrees off the vertical.
    if (m.deck.x <= 0.0 || end || abs(surface.y) > 0.5) {
        return Window(0.0, false, 0.0, 0.0);
    }
    var chart_lo = vec2<f32>(-1.0e9);
    var chart_hi = vec2<f32>(1.0e9);
    if (chart_kind == RECT) {
        chart_lo = chart_bounds.xz;
        chart_hi = chart_bounds.yw;
    } else if (chart_kind == BAND) {
        chart_lo.y = chart_bounds.z;
        chart_hi.y = chart_bounds.w;
    } else if (chart_kind == DISC || chart_kind == DOME) {
        // The square inside the disc: a lying part's end, which is a side.
        chart_hi = vec2<f32>(0.7071 * chart_bounds.x);
        chart_lo = -chart_hi;
    }
    let deck = floor(p.y / m.deck.x);
    let col = floor(p.x / m.deck.y);
    let half = 0.5 * m.pane.xy;
    let center = vec2<f32>((col + 0.5) * m.deck.y, deck * m.deck.x + m.deck.z + half.y);
    let clear = m.chamfer.x + 0.1;
    var lo = cell.lo + clear;
    var hi = cell.hi - clear;
    lo = max(lo, chart_lo + m.deck.w);
    hi = min(hi, chart_hi - m.deck.w);
    if (any(center - half < lo) || any(center + half > hi)) {
        return Window(0.0, false, 0.0, 0.0);
    }
    let inside = half - abs(p - center);
    let cover = clamp(inside / pixel + 0.5, vec2<f32>(0.0), min(vec2<f32>(1.0), 2.0 * half / pixel));
    return Window(cover.x * cover.y, window_lit(face_id, deck, col), col, deck);
}

/// What the eye sees through the glass at `p`, as a share of a lit window's glow: a room the
/// window's column wide, its deck high and `room_m` deep, traced from the glass, with no geometry
/// behind the face at all. Interior mapping.
fn room(p: vec2<f32>, e: vec3<f32>, frame: mat3x3<f32>, face_id: u32, pane: Window) -> vec3<f32> {
    // x and y along the chart, z into it.
    let d = vec3<f32>(-dot(e, frame[0]), -dot(e, frame[1]), max(dot(e, frame[2]), 1.0e-3));
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

    let h = mix_hash(mix_hash(face_id, u32(i32(pane.deck))), u32(i32(pane.col)));
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

/// Stratus cover at `q`, meters on its deck, 0 to 1: soft sheets drawn out along the face's u.
fn stratus(q: vec2<f32>) -> f32 {
    let x = q / STRATUS_SCALE_M;
    let fbm = 0.55 * value_noise(x + 91.0) + 0.3 * value_noise(2.1 * x + 7.0) + 0.15 * value_noise(4.3 * x + 29.0);
    return smoothstep(0.5, 0.72, fbm);
}

/// Signed meters from `q`, in the roof's chart, to its edge, positive inside.
fn inside_roof(q: vec2<f32>) -> f32 {
    if (chart_kind == RECT) {
        let d = min(q - chart_bounds.xz, chart_bounds.yw - q);
        return min(d.x, d.y);
    }
    if (chart_kind == DISC || chart_kind == DOME) {
        return chart_bounds.x - length(q);
    }
    return 1.0e6;
}

/// How far down the eye's ray through the glass, going `d` in the roof's chart with z down, meets
/// the wall dropping from the rim, or `limit` if it reaches that depth first. Walls drop square to
/// the chart, so the roof's own edge is a safe step along the ray's shadow on it.
fn wall_depth(p: vec2<f32>, d: vec3<f32>, limit: f32) -> f32 {
    let slope = d.xy / d.z;
    let run = max(length(slope), 1.0e-3);
    var h = 0.0;
    for (var i = 0; i < WALL_STEPS; i++) {
        let inside = inside_roof(p + slope * h);
        if (i > 0 && inside < 0.02) {
            return h;
        }
        h += max(inside, 0.05) / run;
        if (h >= limit) {
            return limit;
        }
    }
    return limit;
}

/// How deep the forest lies under this roof: a share of its width, so a small roof is not a deep
/// well, up to a most.
fn forest_depth() -> f32 {
    var width = 0.0;
    if (chart_kind == RECT) {
        width = min(chart_bounds.y - chart_bounds.x, chart_bounds.w - chart_bounds.z);
    } else if (chart_kind == DISC || chart_kind == DOME) {
        width = 2.0 * chart_bounds.x;
    }
    if (width <= 0.0) {
        return m.forest.x;
    }
    return min(m.forest.x, m.forest.y * width);
}

/// What a roof pane shows: no light of its own, only the star's and the planet's let in through the
/// glass. A forest [`forest_depth`] down is seen where the eye's ray meets it, so it moves against
/// the frame as the eye does, through air that scatters the light back blue by day and red as the
/// star sets over the roof; the longer the ray's path, the more air hides the forest, so a glancing
/// look sees sky only. Walls drop from the rim to the forest, so a ray never runs on past the hull's
/// edge. A stratus deck `STRATUS_SHARE` of the way down lies over the forest, and shades it
/// where the star's light passes through it on the way down. Nothing by night.
fn sky_below(p: vec2<f32>, frame: mat3x3<f32>, surface: vec3<f32>, e: vec3<f32>, edge: f32) -> vec3<f32> {
    let star = turned(m.to_star);
    let over = dot(star, surface);
    let day = smoothstep(-0.08, 0.25, over);
    let high = smoothstep(0.0, 0.45, over);
    let tint = mix(vec3<f32>(1.0, 0.42, 0.16), m.sky.rgb, high);
    let rayleigh = 0.75 * (1.0 + pow(dot(-star, e), 2.0));
    let air = m.star_color.rgb * m.sky.w * day * mix(vec3<f32>(1.0), tint, 0.7) * rayleigh;

    let d = vec3<f32>(-dot(e, frame[0]), -dot(e, frame[1]), max(dot(e, frame[2]), 0.02));
    let sunlight = m.star_color.rgb * day * mix(vec3<f32>(1.0, 0.6, 0.35), vec3<f32>(1.0), high);
    let star_face = vec3<f32>(dot(star, frame[0]), dot(star, frame[1]), dot(star, frame[2]));
    let floor_m = forest_depth();
    let deck_m = STRATUS_SHARE * floor_m;
    let depth = wall_depth(p, d, floor_m);
    let path = depth / d.z;
    let through = exp(-path / AIR_M);
    var seen: vec3<f32>;
    if (depth < floor_m) {
        // The inside of the hull under the rim, banded by its decks, darker toward the floor.
        let band = select(1.0, 0.5, fract(depth / max(m.deck.x, 3.0)) < 0.06);
        seen = sunlight * WALL_ALBEDO * (0.3 + 0.7 * max(over, 0.0)) * mix(1.0, 0.4, depth / floor_m) * band / PI;
    } else {
        let ground = p + d.xy * path;
        let up_to_deck = (floor_m - deck_m) / max(star_face.z, 0.1);
        let shade = 1.0 - 0.6 * stratus(ground + star_face.xy * up_to_deck);
        seen = canopy(ground, star_face) * sunlight * shade / PI;
    }
    var below = seen * through + air * (1.0 - through);

    if (depth > deck_m) {
        let deck_path = deck_m / d.z;
        let cover = STRATUS_ALPHA * stratus(p + d.xy * deck_path);
        let deck_through = exp(-deck_path / AIR_M);
        let cloud = sunlight * (0.2 + 0.3 * max(over, 0.0)) * mix(0.8, 1.0, high);
        below = mix(below, cloud * deck_through + air * (1.0 - deck_through), cover);
    }
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
/// toward it, so at a box's edge the two strips are one bevel, and on a curved hull a groove. On
/// the roof, toward its chart's edge; on a side, toward its chart's top.
fn rim(p: vec2<f32>, frame: mat3x3<f32>, surface: vec3<f32>, roofed: bool, pixel: f32) -> Rim {
    var dist = 1.0e6;
    var out = vec2<f32>(0.0);
    if (roofed) {
        if (chart_kind == RECT) {
            let to_edge = vec4<f32>(p - chart_bounds.xz, chart_bounds.yw - p);
            dist = min(min(to_edge.x, to_edge.y), min(to_edge.z, to_edge.w));
            out = vec2<f32>(0.0, 1.0);
            if (dist == to_edge.x) { out = vec2<f32>(-1.0, 0.0); }
            else if (dist == to_edge.y) { out = vec2<f32>(0.0, -1.0); }
            else if (dist == to_edge.z) { out = vec2<f32>(1.0, 0.0); }
        } else if (chart_kind == DISC || chart_kind == DOME) {
            dist = chart_bounds.x - length(p);
            out = p / max(length(p), 1.0e-4);
        }
    } else if (chart_kind == RECT || chart_kind == BAND) {
        // The chart's top is whichever v edge lies up.
        let up = dot(frame[1], vec3<f32>(0.0, 1.0, 0.0)) >= 0.0;
        dist = select(p.y - chart_bounds.z, chart_bounds.w - p.y, up);
        out = vec2<f32>(0.0, select(-1.0, 1.0, up));
    }
    let toward = frame[0] * out.x + frame[1] * out.y;
    let half_width = 0.5 * m.roof.z;
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
fn roof(p: vec2<f32>, frame: mat3x3<f32>, surface: vec3<f32>, e: vec3<f32>) -> vec3<f32> {
    let cell = hex(p, m.roof.x);
    let metal = roof_metal(surface, e, surface);

    let f0 = vec3<f32>(0.04);
    let roughness = 0.05;
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - max(dot(surface, e), 0.0), 5.0);
    let half_seam = 0.5 * m.roof.y;
    var pane = (1.0 - fresnel) * sky_below(p, frame, surface, e, cell.z - half_seam);
    let star = turned(m.to_star);
    if (dot(star, surface) > 0.0) {
        pane += m.star_color.rgb * specular(surface, star, e, f0, roughness);
    }
    pane += m.planet_color.rgb * fresnel * planet_seen(reflect(-e, surface), roughness);

    let pixel = max(length(fwidth(p)), 1.0e-4);
    let glass = smoothstep(half_seam - pixel, half_seam + pixel, cell.z);
    return mix(metal, pane, glass);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let surface = normalize(in.world_normal);
    let along = normalize(in.world_tangent - surface * dot(in.world_tangent, surface));
    // Offices and a habitat see a lying part's band as a roof along its top and flanks below, so
    // their decks stand level; bare plate keeps the band whole.
    var frame = mat3x3<f32>(along, cross(surface, along), surface);
    chart_kind = u32(round(in.chart.z));
    chart_bounds = in.bounds;
    var end = in.chart.w > 0.5;
    var p = in.chart.xy;
    if (in.chart.w > 1.5 && (m.deck.x > 0.0 || m.roof.x > 0.0)) {
        let seen = chart_view(in.chart, in.bounds, frame);
        p = seen.p;
        chart_kind = seen.kind;
        chart_bounds = seen.bounds;
        frame = seen.frame;
        end = seen.end;
    }
    // Around a band or ring, stretched to a whole number of grid cells, which are whole numbers of
    // snaps, window columns and decks, so every lattice meets itself at the seam.
    let cell_m = m.grid.x;
    if (chart_kind == BAND || chart_kind == RING) {
        let n = max(round(chart_bounds.x / cell_m), 1.0);
        p.x *= n * cell_m / chart_bounds.x;
        around.x = n * cell_m;
    }
    if (chart_kind == RING) {
        let n = max(round(chart_bounds.y / cell_m), 1.0);
        p.y *= n * cell_m / chart_bounds.y;
        around.y = n * cell_m;
    }
    let pixel = max(length(fwidth(p)), 1.0e-4);
    let pixels = max(fwidth(p), vec2<f32>(1.0e-4));
    let e = normalize(view.world_position - in.world_position);
    // Each chart its own panels, and a rectangle, all of one direction, told apart by which way
    // its u runs: a cube's faces are alike in extent.
    let facing = select(0u, u32(round(dot(along, vec3<f32>(1.0, 3.0, 9.0))) + 13.0), u32(round(in.chart.z)) == RECT);
    let face_id = mix_hash(chart_kind + 8u * facing, u32(in.chart.w) + 2u * bitcast<u32>(chart_bounds.x));
    // The roof is the end that faces up.
    let roofed = m.roof.x > 0.0 && end && surface.y > 0.0;
    var edged = Rim(0.0, surface);
    if (m.roof.x > 0.0) {
        edged = rim(p, frame, surface, roofed, pixel);
    }
    let bevel = roof_metal(edged.normal, e, surface);
    if (roofed) {
        return vec4<f32>(mix(roof(p, frame, surface, e), bevel, edged.share) * m.finish.x, 1.0);
    }
    let cell = panel(p, face_id);

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
    let width = m.chamfer.x;
    let share = (1.0 - smoothstep(width - pixel, width + pixel, edge)) * clamp(width / pixel, 0.0, 1.0);

    let flat_light = lit(n, e, surface, albedo, f0, roughness);
    let wall_light = lit(wall, e, surface, albedo, f0, roughness);
    let plate = mix(flat_light, wall_light, share);
    let pane = window(p, cell, face_id, end, surface, pixels);
    var radiance = plate;
    if (pane.cover > 0.0) {
        let lights = select(m.ambient.rgb * 0.2, m.glow.rgb, pane.lit);
        radiance = mix(plate, glass(n, e, surface, lights * room(p, e, frame, face_id, pane)), pane.cover);
    }
    return vec4<f32>(mix(radiance, bevel, edged.share) * m.finish.x, 1.0);
}
