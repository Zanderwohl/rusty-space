// Launch tubes in a hive: hexagonal cells, each a tube ringed in danger tape under a six-bladed
// iris. An open tube is traced down to its wall, a drone's nose, or the floor. Laid out in the
// part's charts, with a buffed-steel margin two apothems wide so no cell is cut. Look development:
// crates/lc-client/src/looks/tubes.rs.

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
/// Buffed steel's swirls: their pitch, and each one's radius.
const SWIRL_PITCH_M: f32 = 1.2;
const SWIRL_M: f32 = 1.0;
const STEEL: vec3<f32> = vec3<f32>(0.56, 0.57, 0.58);
/// How tight the highlight along the brushing is, and what steel mirrors of the sky.
const BRUSH_EXPONENT: f32 = 90.0;
const SKY: vec3<f32> = vec3<f32>(0.02, 0.025, 0.035);

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

const RECT: u32 = 0u;
const DISC: u32 = 1u;
const BAND: u32 = 2u;
const RING: u32 = 3u;
const DOME: u32 = 4u;

/// `x` modulo `n`, never negative.
fn wrap(x: i32, n: i32) -> i32 {
    return ((x % n) + n) % n;
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

/// What the eye sees down an open tube from `q`, meters from its axis, along `d` in the chart's
/// frame with z into the tube.
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

/// Each point is brushed about the middle of the latest swirl over it, so the star's highlight runs
/// in arcs. Where a swirl is under a few pixels, plain steel.
fn buffed(p: vec2<f32>, frame: mat3x3<f32>, surface: vec3<f32>, e: vec3<f32>, pixel: f32) -> vec3<f32> {
    let g = p / SWIRL_PITCH_M;
    var middle = vec2<f32>(0.0);
    var best = -1.0e9;
    for (var dr = -1; dr <= 1; dr++) {
        let row = floor(g.y) + f32(dr);
        let shift = 0.5 * (row - 2.0 * floor(0.5 * row));
        for (var dc = -1; dc <= 1; dc++) {
            let col = floor(g.x - shift) + f32(dc);
            let c = vec2<f32>(col + shift + 0.5, row + 0.5) * SWIRL_PITCH_M;
            let order = row * 4096.0 + col;
            if (distance(p, c) < SWIRL_M && order > best) {
                best = order;
                middle = c;
            }
        }
    }
    let off = p - middle;
    let brush = normalize(frame * vec3<f32>(select(vec2<f32>(1.0, 0.0), vec2<f32>(-off.y, off.x) / length(off), length(off) > 1.0e-4), 0.0));
    // The finest rings of the turning, a shade apart, gone before they alias.
    let rings = 1.0 + 0.04 * sin(2.0 * PI * length(off) / 0.03) * (1.0 - smoothstep(0.004, 0.012, pixel));
    let anisotropic = 1.0 - smoothstep(0.1, 0.5, pixel / SWIRL_PITCH_M);

    let star = turned(m.to_star);
    let n = surface;
    let r = reflect(-e, n);
    let fresnel = STEEL + (1.0 - STEEL) * pow(1.0 - max(dot(n, e), 0.0), 5.0);
    var radiance = SKY * fresnel * mix(0.4, 1.0, 0.5 + 0.5 * r.y);
    radiance += m.planet_color.rgb * fresnel * planet_seen(r, 0.35);
    if (dot(star, surface) > 0.0) {
        let ndl = max(dot(n, star), 0.0);
        let h = normalize(star + e);
        let along = dot(brush, h);
        let streak = pow(max(1.0 - along * along, 0.0), 0.5 * BRUSH_EXPONENT) * (BRUSH_EXPONENT + 2.0) / (8.0 * PI);
        // Unresolved, the swirls' streaks average to a sheen over the whole face.
        let even = specular(n, star, e, STEEL, 0.4) + 0.06 * fresnel * ndl;
        radiance += m.star_color.rgb * (mix(even, fresnel * streak * ndl * 0.25, anisotropic) + 0.03 * STEEL * ndl);
    }
    return radiance * rings;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let surface = normalize(in.world_normal);
    let along = normalize(in.world_tangent - surface * dot(in.world_tangent, surface));
    let frame = mat3x3<f32>(along, cross(surface, along), surface);
    let p = in.chart.xy;
    let kind = u32(round(in.chart.z));
    let bounds = in.bounds;
    let e = normalize(view.world_position - in.world_position);
    let pixel = max(length(fwidth(p)), 1.0e-4);
    let star = turned(m.to_star);

    // A whole number of cells around a wrap; down a ring's tube, of pairs of rows.
    var w = m.cell.x;
    var across = 1;
    if (kind == BAND || kind == RING) {
        across = max(i32(round(bounds.x / w)), 1);
        w = bounds.x / f32(across);
    }
    var squash = 1.0;
    if (kind == RING) {
        let pairs = max(round(bounds.y / (1.7320508 * w)), 1.0);
        squash = bounds.y / (1.7320508 * w * pairs);
    }
    let h = hex(vec2<f32>(p.x, p.y / squash), w);
    let off = vec2<f32>(h.x, h.y * squash);
    let center = p - off;
    // Two apothems, a cell's width, between a kept cell's middle and the chart's edge.
    var kept = true;
    if (kind == RECT) {
        kept = all(center >= bounds.xz + vec2<f32>(w)) && all(center <= bounds.yw - vec2<f32>(w));
    } else if (kind == DISC || kind == DOME) {
        kept = length(center) <= bounds.x - w;
    } else if (kind == BAND) {
        kept = center.y >= bounds.z + w && center.y <= bounds.w - w;
    }
    if (!kept) {
        return vec4<f32>(buffed(p, frame, surface, e, pixel) * m.finish.x, 1.0);
    }

    // Named by its column, taken around a band or ring, so the cells either side of a seam agree.
    var column = i32(round(2.0 * center.x / w));
    if (kind == BAND || kind == RING) {
        column = wrap(column, 2 * across);
    }
    let seed = mix_hash(u32(m.finish.y), kind + 7u * u32(i32(round(bounds.x))));
    let id = mix_hash(mix_hash(seed, u32(column)), u32(i32(round(center.y * 4.0))));
    let base = m.paint.rgb * mix(0.92, 1.08, unit(pcg(id + 3u)));
    var radiance = lit(surface, e, surface, base);

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

        let s = (phi * (r + 0.5 * tape) + (d - r)) / STRIPE_M;
        let stripe_aa = 2.0 * pixel / STRIPE_M;
        let white = smoothstep(0.5 - stripe_aa, 0.5 + stripe_aa, abs(fract(0.5 * s) * 2.0 - 1.0));
        // Fading into the plate as the tape narrows under a pixel, so a far hive is gray, not pink.
        let tape_color = mix(mix(TAPE_RED, TAPE_WHITE, white), base, smoothstep(0.25 * tape, tape, pixel));
        let lip = clamp(1.0 - (d - r) / LIP_M, 0.0, 1.0);
        let tape_n = normalize(frame * vec3<f32>(-radial * lip * steady(1.2, 0.5 * LIP_M, pixel), 1.0));
        let tape_light = lit(tape_n, e, surface, tape_color);
        radiance = mix(radiance, tape_light, 1.0 - smoothstep(r + tape - pixel, r + tape + pixel, d));

        if (d < r + pixel) {
            let state = unit(pcg(id + 5u));
            let open = m.tube.z;
            let aperture = select(select(0.0, 0.5 * r, state < open + 0.12), 0.97 * r, state < open);
            // Each blade leans so it overlaps the next.
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
                let into = vec3<f32>(-dot(e, frame[0]), -dot(e, frame[1]), max(dot(e, surface), 1.0e-3));
                let daylight = m.star_color.rgb * max(dot(star, surface), 0.0) * 0.2 + m.ambient.rgb;
                let inside = down_tube(off, into, loaded, daylight, pixel);
                radiance = mix(radiance, inside, 1.0 - smoothstep(aperture - pixel, aperture + pixel, d));
            }
        }
    }
    return vec4<f32>(radiance * m.finish.x, 1.0);
}
