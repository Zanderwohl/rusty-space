// Azteced plate: rectangles cut from a staggered grid by snapped random splits, each panel one
// shade and one flat normal tipped a few degrees, with chamfered trenches between. Laid out in a
// part's chart (crate::charts in lc-client): `p` is meters there, `frame` its u, its v and the
// surface's normal, and `around` how far u and v run before they repeat, a whole number of grid
// cells, 0 where they do not. Shared by hull.wgsl; aztec.wgsl is its development copy.

#import "shaders/hash.wgsl"::{mix_hash, pcg, unit}

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

struct Panel {
    lo: vec2<f32>,
    hi: vec2<f32>,
    id: u32,
}

const SPLITS: u32 = 6u;

/// The panel holding `p`, meters on the face. `grid` is `(cell_m, min_panel_m, snap_m, _)`: cells
/// staggered by row so the grid's lines do not run the face's length, split on a snapped random
/// line, mostly across the longer side, until a split would leave a panel under the minimum or
/// the hash says stop. Snapped bounds are exact, so they name the panel whichever pixel asks.
fn panel(p: vec2<f32>, face_id: u32, grid: vec4<f32>, seed: u32, around: vec2<f32>) -> Panel {
    let cell = grid.x;
    let snap = grid.z;
    let face_seed = mix_hash(face_id, seed);
    let row = floor(p.y / cell);
    let row_id = around_index(row, cell, around.y);
    let shift = round(unit(mix_hash(face_seed, row_id)) * cell / snap) * snap;
    let col = floor((p.x - shift) / cell);
    var lo = vec2<f32>(col * cell + shift, row * cell);
    var hi = lo + vec2<f32>(cell);
    var id = mix_hash(mix_hash(face_seed, row_id), around_index(col, cell, around.x));
    for (var depth = 0u; depth < SPLITS; depth++) {
        let h = pcg(id);
        let size = hi - lo;
        if (depth >= 1u && unit(h) < 0.22 * f32(depth)) {
            break;
        }
        let longer_x = size.x >= size.y;
        let across_x = select(!longer_x, longer_x, unit(pcg(h)) < 0.8);
        let span = select(size.y, size.x, across_x);
        if (span < 2.0 * grid.y) {
            break;
        }
        let share = mix(0.2, 0.8, unit(pcg(h + 1u)));
        let at = select(lo.y, lo.x, across_x) + max(round(share * span / snap) * snap, grid.y);
        let cut = min(at, select(hi.y, hi.x, across_x) - grid.y);
        if (across_x) {
            if (p.x < cut) { hi.x = cut; } else { lo.x = cut; }
        } else {
            if (p.y < cut) { hi.y = cut; } else { lo.y = cut; }
        }
        let bound = round(vec4<f32>(lo, hi) / snap);
        let snapped = vec4<u32>(
            around_index(bound.x, snap, around.x),
            around_index(bound.y, snap, around.y),
            around_index(bound.z, snap, around.x),
            around_index(bound.w, snap, around.y),
        );
        id = mix_hash(mix_hash(snapped.x ^ (snapped.y << 16u), snapped.z ^ (snapped.w << 16u)), face_seed);
    }
    return Panel(lo, hi, id);
}

/// The panel's normal in the face's frame: tipped by up to `tilt` radians, uniform over that disc.
fn tipped(id: u32, tilt: f32) -> vec3<f32> {
    let r = tilt * sqrt(unit(pcg(id + 7u)));
    let a = 6.2831853 * unit(pcg(id + 11u));
    return vec3<f32>(sin(r) * cos(a), sin(r) * sin(a), cos(r));
}

/// Mostly gray, `brown` of panels a little browner, every panel a shade off its neighbors.
fn paint(id: u32, brown: f32) -> vec3<f32> {
    let gray = mix(0.36, 0.46, unit(pcg(id + 3u)));
    let browner = unit(pcg(id + 5u)) < brown;
    let tint = select(vec3<f32>(1.0), mix(vec3<f32>(1.0), vec3<f32>(1.06, 0.95, 0.82), 0.5 + 0.5 * unit(pcg(id + 9u))), browner);
    return gray * tint;
}

struct Plate {
    /// The panel's own normal, and the chamfer's where the point is on one, in the surface's frame.
    normal: vec3<f32>,
    wall: vec3<f32>,
    /// What of the pixel the chamfer covers.
    wall_share: f32,
    color: vec3<f32>,
    panel: Panel,
}

/// The plate at `p` in a chart named `face_id`, `pixel` meters across. `grid` as [`panel`];
/// `finish` is `(tilt_rad, brown, chamfer_m, lean_rad)`. A panel's tilt fades as the panel
/// shrinks under a few pixels, so it does not sparkle.
fn plate(p: vec2<f32>, frame: mat3x3<f32>, face_id: u32, around: vec2<f32>, grid: vec4<f32>, finish: vec4<f32>, pixel: f32, seed: u32) -> Plate {
    let cell = panel(p, face_id, grid, seed, around);
    let keep = 1.0 - smoothstep(0.1, 0.5, pixel / grid.y);
    let n = normalize(frame * mix(vec3<f32>(0.0, 0.0, 1.0), tipped(cell.id, finish.x), keep));

    // A chamfer slopes down into the seam from each side, so the two walls of the trench face
    // each other. Its normal leans toward the nearest edge, which miters the corners.
    let to_edge = vec4<f32>(p.x - cell.lo.x, cell.hi.x - p.x, p.y - cell.lo.y, cell.hi.y - p.y);
    let edge = min(min(to_edge.x, to_edge.y), min(to_edge.z, to_edge.w));
    var outward = vec2<f32>(0.0, 1.0);
    if (edge == to_edge.x) { outward = vec2<f32>(-1.0, 0.0); }
    else if (edge == to_edge.y) { outward = vec2<f32>(1.0, 0.0); }
    else if (edge == to_edge.z) { outward = vec2<f32>(0.0, -1.0); }
    let wall = normalize(n * cos(finish.w) + (frame[0] * outward.x + frame[1] * outward.y) * sin(finish.w));
    let width = finish.z;
    let share = (1.0 - smoothstep(width - pixel, width + pixel, edge)) * clamp(width / pixel, 0.0, 1.0);
    return Plate(n, wall, share, paint(cell.id, finish.y), cell);
}
