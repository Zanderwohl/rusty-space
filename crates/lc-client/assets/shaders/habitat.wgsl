// The habitat's pieces that need no binding: office windows on a plate's sides and the rooms traced
// behind them, and the roof's glass hexes over a forest and a stratus deck. Light is in the
// caller's units, handed in. Shared by hull.wgsl; aztec.wgsl is its development copy.

#import "shaders/hash.wgsl"::{mix_hash, pcg, unit}
#import "shaders/panels.wgsl"::{Panel, around_index}

struct Window {
    cover: f32,
    lit: bool,
    /// Which column and deck, which are its room's.
    col: f32,
    deck: f32,
}

/// Whether the window at `deck`, `col` is lit: mostly in runs along its row, at two lengths so
/// runs vary, with a few dark gaps in them and a few lone windows lit outside.
fn window_lit(face_id: u32, deck: u32, col: i32, share: f32) -> bool {
    let row = mix_hash(face_id, deck);
    let short = i32(floor((f32(col) + 97.0 * unit(pcg(row + 1u))) / 5.0));
    let long = i32(floor((f32(col) + 97.0 * unit(pcg(row + 2u))) / 11.0));
    let run = unit(mix_hash(row + 3u, u32(short))) < 0.6 * share
        || unit(mix_hash(row + 4u, u32(long))) < 0.4 * share;
    let own = unit(mix_hash(row + 5u, u32(col)));
    return select((own < 0.06), (own > 0.12), run);
}

/// The window over `p`, meters in a side's chart, if one fits wholly inside its panel clear of
/// `clear` meters and inside `lo`..`hi`; its cover is what of the pixel it fills. `deck` is
/// `(deck_m, pitch_m, sill_m, _)` and `pane` `(width_m, height_m, ...)`. Its row and column are
/// taken around the chart's wrap, `around`, as [`around_index`] does.
fn window(p: vec2<f32>, cell: Panel, face_id: u32, deck: vec4<f32>, pane: vec2<f32>, clear: f32, lo: vec2<f32>, hi: vec2<f32>, lit_share: f32, around: vec2<f32>, pixel: vec2<f32>) -> Window {
    let row = floor(p.y / deck.x);
    let col = floor(p.x / deck.y);
    let half = 0.5 * pane;
    let center = vec2<f32>((col + 0.5) * deck.y, row * deck.x + deck.z + half.y);
    let fits_lo = max(cell.lo + clear, lo);
    let fits_hi = min(cell.hi - clear, hi);
    if (any(center - half < fits_lo) || any(center + half > fits_hi)) {
        return Window(0.0, false, 0.0, 0.0);
    }
    let inside = half - abs(p - center);
    let cover = clamp(inside / pixel + 0.5, vec2<f32>(0.0), min(vec2<f32>(1.0), 2.0 * half / pixel));
    let lit = window_lit(face_id, around_index(row, deck.x, around.y), i32(around_index(col, deck.y, around.x)), lit_share);
    return Window(cover.x * cover.y, lit, col, row);
}

const DESK_TOP_M: f32 = 0.75;
const DESK_FROM_M: f32 = 1.2;
const DESK_DEPTH_M: f32 = 0.8;

/// What the eye sees through the glass at `p`, as a share of a lit window's glow: a room the
/// window's column wide, its deck high and `depth_m` deep, traced from the glass, with no geometry
/// behind the face at all. `e` is toward the eye.
fn room(p: vec2<f32>, e: vec3<f32>, frame: mat3x3<f32>, face_id: u32, pane: Window, deck: vec4<f32>, depth_m: f32) -> vec3<f32> {
    // x and y along the chart, z into it.
    let d = vec3<f32>(-dot(e, frame[0]), -dot(e, frame[1]), max(dot(e, frame[2]), 1.0e-3));
    let lo = vec2<f32>(pane.col * deck.y, pane.deck * deck.x);
    let hi = lo + deck.yx;
    let toward = select(lo, hi, d.xy > vec2<f32>(0.0));
    // Signed as `toward` was chosen, or a ray square to an axis would meet a wall behind the glass.
    let tiny = select(vec2<f32>(-1.0e-6), vec2<f32>(1.0e-6), d.xy > vec2<f32>(0.0));
    let safe = select(d.xy, tiny, abs(d.xy) < vec2<f32>(1.0e-6));
    let t_side = (toward - p) / safe;
    let t_back = depth_m / d.z;
    let t = min(min(t_side.x, t_side.y), t_back);
    let hit = vec3<f32>(p + d.xy * t, d.z * t);

    let h = mix_hash(mix_hash(face_id, u32(i32(pane.deck))), u32(i32(pane.col)));
    let tint = mix(vec3<f32>(0.82, 0.88, 1.0), vec3<f32>(1.0, 0.9, 0.76), unit(pcg(h)));
    let desk_y = lo.y + DESK_TOP_M;
    let t_front = DESK_FROM_M / d.z;
    if (t_front < t && p.y + d.y * t_front < desk_y) {
        return tint * 0.08;
    }
    if (d.y < 0.0) {
        let t_top = (desk_y - p.y) / d.y;
        if (t_top < t && d.z * t_top < DESK_FROM_M + DESK_DEPTH_M) {
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
        let across = abs(hit.x - 0.5 * (lo.x + hi.x));
        let lamp = across < 0.35 && abs(hit.z - 0.5 * depth_m) < 0.6;
        return select(vec3<f32>(0.8), vec3<f32>(3.0), lamp);
    }
    return tint * 0.15 * (1.0 - 0.5 * clamp(abs(hit.z - 0.5 * depth_m) / depth_m, 0.0, 1.0));
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

const CROWN_M: f32 = 5.0;
const GROVE_M: f32 = 40.0;

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

const STRATUS_SCALE_M: vec2<f32> = vec2<f32>(35.0, 12.0);

/// Stratus cover at `q`, meters on its deck, 0 to 1: soft sheets drawn out along the face's u.
fn stratus(q: vec2<f32>) -> f32 {
    let x = q / STRATUS_SCALE_M;
    let fbm = 0.55 * value_noise(x + 91.0) + 0.3 * value_noise(2.1 * x + 7.0) + 0.15 * value_noise(4.3 * x + 29.0);
    return smoothstep(0.5, 0.72, fbm);
}
