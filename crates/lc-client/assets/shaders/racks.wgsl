// Server racks: servers a deck tall between rounded posts, each its own shade, slightly tipped, and
// wearing a baked glass face that mirrors by day and glows by night; on a part's ends, a data hall
// from above. Laid out in the part's charts, a whole number of racks around a wrap. Look
// development: crates/lc-client/src/looks/racks/mod.rs.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::{globals, view},
    view_transformations::position_world_to_clip,
}
#import "shaders/hash.wgsl"::{pcg, mix_hash, unit}
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
    /// `(roughness, metallic, glass_f0, glass_roughness)`.
    surface: vec4<f32>,
    /// `(width_m, height_m, count, day_glow)`: a face's extent, centered on its server, how many
    /// faces there are, and on a face the star lights, what of the glow is left.
    face: vec4<f32>,
    /// Lit glass, linear.
    glow: vec4<f32>,
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
/// A face a layer: `(cover, glow, 0, 1)`, row 0 at the face's top.
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var faces: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var faces_sampler: sampler;

const PI: f32 = 3.14159265;
/// A sunken aisle's wall, across the face.
const AISLE_WALL_M: f32 = 0.3;
/// A fan's grille: rings this far apart, leaning this far as a tangent, and its hub's share of it.
const RING_M: f32 = 0.25;
const RING_LEAN: f32 = 0.5;
const HUB_SHARE: f32 = 0.22;
/// What face glass mirrors of the sky: dim above, dimmer below.
const SKY: vec3<f32> = vec3<f32>(0.012, 0.016, 0.024);

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

const BAND: u32 = 2u;
const RING: u32 = 3u;

/// `x` modulo `n`, never negative.
fn wrap(x: i32, n: i32) -> i32 {
    return ((x % n) + n) % n;
}

/// `pitch` made to fit `period` a whole number of times, and that number; unchanged, with no
/// count, for no period.
fn fitted(pitch: f32, period: f32) -> vec2<f32> {
    if (period <= 0.0) {
        return vec2<f32>(pitch, 0.0);
    }
    let n = max(round(period / pitch), 1.0);
    return vec2<f32>(period / n, n);
}

/// A normal in the face's frame, tipped by up to `tilt` radians, uniform over that disc.
fn tipped(id: u32, tilt: f32) -> vec3<f32> {
    let r = tilt * sqrt(unit(pcg(id + 7u)));
    let a = 2.0 * PI * unit(pcg(id + 11u));
    return vec3<f32>(sin(r) * cos(a), sin(r) * sin(a), cos(r));
}

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

/// Face glass of normal `n`: dark, all reflection.
fn glass(n: vec3<f32>, e: vec3<f32>, face_n: vec3<f32>) -> vec3<f32> {
    let f0 = vec3<f32>(m.surface.z);
    let roughness = m.surface.w;
    let r = reflect(-e, n);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - max(dot(n, e), 0.0), 5.0);
    var radiance = SKY * fresnel * mix(0.3, 1.0, 0.5 + 0.5 * r.y);
    radiance += m.planet_color.rgb * fresnel * planet_seen(r, roughness);
    let star = turned(m.to_star);
    if (dot(star, face_n) > 0.0) {
        radiance += m.star_color.rgb * specular(n, star, e, f0, roughness);
    }
    return radiance;
}

/// `periods` are u's and v's, 0 where they do not wrap. `dx` and `dy` are the chart's screen
/// derivatives, taken where control flow is uniform.
fn side(p: vec2<f32>, seed: u32, periods: vec2<f32>, frame: mat3x3<f32>, surface: vec3<f32>, e: vec3<f32>, pixel: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec3<f32> {
    let across = fitted(m.rack.x, periods.x);
    let up = fitted(m.rack.y, periods.y);
    let rack = across.x;
    let server = up.x;
    var cell = floor(p / vec2<f32>(rack, server));
    // Named around a wrap, so the servers either side of a seam agree.
    let named = vec2<i32>(
        select(i32(cell.x), wrap(i32(cell.x), i32(across.y)), across.y > 0.0),
        select(i32(cell.y), wrap(i32(cell.y), i32(up.y)), up.y > 0.0),
    );
    let id = mix_hash(mix_hash(seed, u32(named.x)), u32(named.y));
    let base = shade(id);
    let n = normalize(frame * tipped(id, m.lean.z));
    var radiance = lit(n, e, surface, base);

    // Scaled with the server, so it keeps its margin where the server was fitted to a wrap.
    let size = m.face.xy * vec2<f32>(rack, server) / m.rack.xy;
    let local = p - cell * vec2<f32>(rack, server) - 0.5 * (vec2<f32>(rack, server) - size);
    let uv = vec2<f32>(local.x, size.y - local.y) / size;
    let flip = vec2<f32>(1.0, -1.0) / size;
    let layer = i32(pcg(id + 13u) % u32(m.face.z));
    let texel = textureSampleGrad(faces, faces_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), layer, dx * flip, dy * flip);
    let inside = select(0.0, 1.0, all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)));
    let day = smoothstep(-0.05, 0.2, dot(turned(m.to_star), surface));
    let glowing = m.glow.rgb * texel.g * mix(1.0, m.face.w, day);
    radiance = mix(radiance, glass(n, e, surface) + glowing, texel.r * inside);

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

/// An end: rows of rack tops along u, each a fan in every rack's width, between aisles.
fn hall(p: vec2<f32>, seed: u32, frame: mat3x3<f32>, surface: vec3<f32>, e: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    let rack = m.rack.x;
    let depth = m.hall.x;
    let pitch = depth + m.hall.y;
    let row = floor(p.y / pitch);
    let across = p.y - row * pitch;
    let id = mix_hash(mix_hash(seed, u32(i32(row))), u32(i32(floor(p.x / rack))));
    let base = shade(id);

    let middle = vec2<f32>((floor(p.x / rack) + 0.5) * rack, row * pitch + 0.5 * depth);
    var top = lit(normalize(frame * tipped(id, m.lean.z)), e, surface, base);
    let gx = offset(p.x, rack);
    let gh = 0.5 * m.rack.w;
    let groove = normalize(vec3<f32>(-sign(gx) * steady(m.lean.y, gh, pixel.x), 0.0, 1.0));
    top = mix(top, lit(frame * groove, e, surface, 0.35 * base), cover(gx, gh, pixel.x));

    // The fan: a grille of rings about a hub.
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
    let along = normalize(in.world_tangent - surface * dot(in.world_tangent, surface));
    // A lying part's band seen as a hall along its top and racks standing up its flanks.
    let seen = chart_view(in.chart, in.bounds, mat3x3<f32>(along, cross(surface, along), surface));
    let frame = seen.frame;
    let p = seen.p;
    let kind = seen.kind;
    let e = normalize(view.world_position - in.world_position);
    let pixel = max(fwidth(p), vec2<f32>(1.0e-4));
    let dx = dpdx(p);
    let dy = dpdy(p);
    // A cube's faces are alike in extent, so a rectangle is also told apart by its u's direction.
    let facing = select(0u, u32(round(dot(along, vec3<f32>(1.0, 3.0, 9.0))) + 13.0), u32(round(in.chart.z)) == 0u);
    let seed = mix_hash(mix_hash(u32(m.finish.y), kind + 8u * facing), bitcast<u32>(seen.bounds.x + 2.0 * seen.bounds.w) ^ u32(in.chart.w));
    var periods = vec2<f32>(0.0);
    if (kind == BAND || kind == RING) {
        periods.x = seen.bounds.x;
    }
    if (kind == RING) {
        periods.y = seen.bounds.y;
    }
    var radiance: vec3<f32>;
    if (seen.end) {
        radiance = hall(p, seed, frame, surface, e, pixel);
    } else {
        radiance = side(p, seed, periods, frame, surface, e, pixel, dx, dy);
    }
    return vec4<f32>(radiance * m.finish.x, 1.0);
}
