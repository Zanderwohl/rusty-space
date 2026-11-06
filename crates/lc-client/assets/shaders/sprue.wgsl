// A layered sprue: an opaque base and two cutout layers of pipe, flat and normal-mapped, standing
// off one another. Lamps are baked into `light`; only the star is lit here. Laid out in the part's
// own charts (crate::charts in lc-client), each layer the part grown by its height, its chart
// brought back to the base's so the three layers' pipes stand over one another, and a whole
// number of tiles around a band or ring.
// Look development: crates/lc-client/src/looks/sprue.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::{globals, view},
    view_transformations::position_world_to_clip,
}

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

struct SprueUniform {
    /// `(layer, tile_m, 0, 0)`.
    layer: vec4<f32>,
    /// Meters over the base: layer 1, layer 2, and two the bake used.
    heights: vec4<f32>,
    /// The baked light's unit: linear color times power at a meter.
    lamp_color: vec4<f32>,
    /// Toward the star, and its turn in `w`.
    to_star: vec4<f32>,
    star_color: vec4<f32>,
    ambient: vec4<f32>,
    /// `(exposure, gloss, specular, 0)`.
    finish: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: SprueUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var tiles: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var tile_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var light: texture_2d_array<f32>;

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

const PI: f32 = 3.14159265;
const BAND: u32 = 2u;
const RING: u32 = 3u;
const DOME: u32 = 4u;

/// Tiles across a chart: their size each way, and how many fit around a wrap, 0 for none.
struct Tiling {
    size: vec2<f32>,
    around: vec2<f32>,
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

/// A tile cell taken around a wrap, so the cells either side of a seam are one.
fn wrapped(c: vec2<f32>, around: vec2<f32>) -> vec2<f32> {
    return select(c, c - around * floor(c / max(around, vec2<f32>(1.0))), around > vec2<f32>(0.0));
}

fn hash2(p: f32) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

/// Whether the edge of tile cells at `at` on `axis` (0 vertical, 1 horizontal) takes its
/// second choice of lamps.
fn edge(at: vec2<f32>, axis: f32, id: f32) -> u32 {
    var p3 = fract(vec3<f32>(at, id * 7.0 + axis) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return u32(fract((p3.x + p3.y) * p3.z) < 0.5);
}

/// Sixteen variants, as looks::sprue::VARIANTS, of three layers each.
fn variant_layer(uv: vec2<f32>, k: u32, id: f32, around: vec2<f32>) -> u32 {
    let c = floor(uv);
    let variant = edge(wrapped(c, around), 0.0, id)
        + 2u * edge(wrapped(c + vec2<f32>(1.0, 0.0), around), 0.0, id)
        + 4u * edge(wrapped(c, around), 1.0, id)
        + 8u * edge(wrapped(c + vec2<f32>(0.0, 1.0), around), 1.0, id);
    return variant * 3u + k;
}

fn layer_height(k: u32) -> f32 {
    if (k == 1u) { return m.heights.x; }
    if (k == 2u) { return m.heights.y; }
    return 0.0;
}

/// What of the star reaches height `z` at `p`, past the cutout layers above.
fn starlit(p: vec2<f32>, z: f32, toward: vec3<f32>, id: f32, tiling: Tiling) -> f32 {
    var through = 1.0;
    for (var k = 1u; k <= 2u; k++) {
        let zk = layer_height(k);
        if (zk > z) {
            let at = p + toward.xy * (zk - z) / toward.z;
            let tile_uv = at / tiling.size;
            let cover = textureSampleLevel(tiles, tile_sampler, fract(tile_uv), variant_layer(tile_uv, k, id, tiling.around), 1.0).a;
            through *= 1.0 - smoothstep(0.35, 0.65, cover);
        }
    }
    return through;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let surface = normalize(in.world_normal);
    let along = normalize(in.world_tangent - surface * dot(in.world_tangent, surface));
    let frame = mat3x3<f32>(along, cross(surface, along), surface);
    let k = u32(m.layer.x + 0.5);
    let kind = u32(round(in.chart.z));
    let lift = layer_height(k);

    // Back to the base's chart: a layer grown round a curve measures its arcs that much farther out.
    var p = in.chart.xy;
    var periods = vec2<f32>(0.0);
    if (kind == BAND || kind == RING) {
        let base = in.bounds.x - 2.0 * PI * lift * f32(kind == BAND);
        p.x *= base / in.bounds.x;
        periods.x = base;
    }
    if (kind == RING) {
        let base = in.bounds.y - 2.0 * PI * lift;
        p.y *= base / in.bounds.y;
        periods.y = base;
    }
    if (kind == DOME) {
        p *= (in.bounds.y - lift) / in.bounds.y;
    }
    let across = fitted(m.layer.y, periods.x);
    let up = fitted(m.layer.y, periods.y);
    let tiling = Tiling(vec2<f32>(across.x, up.x), vec2<f32>(across.y, up.y));

    // Each chart its own tiles, alike on all three layers; a rectangle, all of one direction, told
    // apart by which way its u runs, as a cube's faces are alike in extent.
    let facing = select(0.0, round(dot(along, vec3<f32>(1.0, 3.0, 9.0))), kind == 0u);
    let id = f32(kind) * 31.0 + in.chart.w * 7.0 + facing;
    // Each chart starts its tile somewhere else, so neighbors do not repeat edge to edge.
    p += hash2(id) * tiling.size;
    let uv = p / tiling.size;
    let layer = variant_layer(uv, k, id, tiling.around);
    // Clamped within the tile: its neighbor is another variant, not its own far edge. Gradients
    // from the continuous uv, since fract's jump at the seam would select the smallest mip.
    let within = fract(uv);
    let ddx = dpdx(uv);
    let ddy = dpdy(uv);
    let texel = textureSampleGrad(tiles, tile_sampler, within, layer, ddx, ddy);
    let baked = textureSampleGrad(light, tile_sampler, within, layer, ddx, ddy).rgb;
    if (k > 0u && texel.a < 0.5) {
        discard;
    }
    let tn = vec2<f32>(texel.r, texel.g) * 2.0 - 1.0;
    let n = normalize(frame * vec3<f32>(tn, sqrt(max(1.0 - dot(tn, tn), 0.0))));
    let albedo = texel.b;

    var radiance = m.ambient.rgb * albedo + baked * m.lamp_color.rgb;

    let star = turned(m.to_star);
    let toward = vec3<f32>(dot(star, frame[0]), dot(star, frame[1]), dot(star, surface));
    if (toward.z > 0.0) {
        let to_eye = normalize(view.world_position - in.world_position);
        let half_way = normalize(star + to_eye);
        let spec = m.finish.z * pow(max(dot(n, half_way), 0.0), m.finish.y);
        let ndl = max(dot(n, star), 0.0);
        radiance += m.star_color.rgb * starlit(p, lift, toward, id, tiling) * ndl * (albedo + spec);
    }

    return vec4<f32>(radiance * m.finish.x, 1.0);
}
