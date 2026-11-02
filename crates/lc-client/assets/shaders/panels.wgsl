// Azteced plate: rectangles cut from a staggered grid by snapped random splits, each panel one
// shade and one flat normal tipped a few degrees, with chamfered trenches between. A point is
// mapped by the box face its normal faces most, as a cube map would, with `up` the frame's
// vertical so a side's v runs up it. Shared by hull.wgsl; aztec.wgsl is its development copy.

#import "shaders/hash.wgsl"::{mix_hash, pcg, unit}

struct PanelFace {
    u: vec3<f32>,
    v: vec3<f32>,
    n: vec3<f32>,
    id: u32,
}

/// The axis `normal` is nearest, signed.
fn dominant(normal: vec3<f32>) -> vec3<f32> {
    let a = abs(normal);
    if (a.x >= a.y && a.x >= a.z) { return vec3<f32>(sign(normal.x), 0.0, 0.0); }
    if (a.y >= a.z) { return vec3<f32>(0.0, sign(normal.y), 0.0); }
    return vec3<f32>(0.0, 0.0, sign(normal.z));
}

fn panel_face(normal: vec3<f32>, up: vec3<f32>) -> PanelFace {
    let n = dominant(normal);
    let across = select(up, vec3<f32>(up.y, up.z, up.x), abs(dot(n, up)) > 0.9);
    let u = normalize(cross(across, n));
    return PanelFace(u, cross(n, u), n, u32(dot(round(n), vec3<f32>(1.0, 3.0, 9.0)) + 13.0));
}

/// The face's u and v laid into the surface's tangent plane, and the surface's own normal.
fn tangent_frame(f: PanelFace, surface: vec3<f32>) -> mat3x3<f32> {
    let t = normalize(f.u - surface * dot(f.u, surface));
    return mat3x3<f32>(t, cross(surface, t), surface);
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
fn panel(p: vec2<f32>, face_id: u32, grid: vec4<f32>, seed: u32) -> Panel {
    let cell = grid.x;
    let snap = grid.z;
    let face_seed = mix_hash(face_id, seed);
    let row = floor(p.y / cell);
    let shift = round(unit(mix_hash(face_seed, u32(i32(row)))) * cell / snap) * snap;
    let col = floor((p.x - shift) / cell);
    var lo = vec2<f32>(col * cell + shift, row * cell);
    var hi = lo + vec2<f32>(cell);
    var id = mix_hash(mix_hash(face_seed, u32(i32(row))), u32(i32(col)));
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
        let snapped = vec4<u32>(vec4<i32>(round(vec4<f32>(lo, hi) / snap)));
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
    face: PanelFace,
    panel: Panel,
    /// Meters on the face.
    p: vec2<f32>,
}

/// The plate at `pos` with surface normal `surface`, `pixel` meters across. `grid` as [`panel`];
/// `finish` is `(tilt_rad, brown, chamfer_m, lean_rad)`. A panel's tilt fades as the panel
/// shrinks under a few pixels, so it does not sparkle.
fn plate(pos: vec3<f32>, surface: vec3<f32>, up: vec3<f32>, grid: vec4<f32>, finish: vec4<f32>, pixel: f32, seed: u32) -> Plate {
    let f = panel_face(surface, up);
    let frame = tangent_frame(f, surface);
    let p = vec2<f32>(dot(pos, f.u), dot(pos, f.v));
    let cell = panel(p, f.id, grid, seed);
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
    return Plate(n, wall, share, paint(cell.id, finish.y), f, cell, p);
}
