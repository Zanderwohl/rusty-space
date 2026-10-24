// A layered sprue: an opaque plated base and two cutout layers of pipe over it, each flat and
// normal-mapped, standing off one another. Look development only:
// crates/lc-client/examples/sprue_void.
//
// The lamps between the layers are baked into `light` when the tile is made; only the star is
// lit here, since it moves as the ship turns. Each face is mapped in its own frame (u, v, n),
// meters, from world position, so the three layers of one face share coordinates. The mesh
// must be axis-aligned boxes.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
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
}

struct SprueUniform {
    /// `(layer, tile_m, 0, 0)`.
    layer: vec4<f32>,
    /// Meters over the base: layer 1, layer 2, and two the bake used.
    heights: vec4<f32>,
    /// What the baked light is in units of: linear color times power at a meter.
    lamp_color: vec4<f32>,
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
    id: f32,
}

fn face(normal: vec3<f32>) -> Face {
    let n = normalize(normal);
    let up = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.z) > 0.9);
    let u = normalize(cross(up, n));
    return Face(u, cross(n, u), n, dot(round(n), vec3<f32>(1.0, 3.0, 9.0)));
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

/// The arrays' layer for layer `k` of the tile `uv` falls in: sixteen variants, as
/// sprue_void's VARIANTS, of three layers each.
fn variant_layer(uv: vec2<f32>, k: u32, id: f32) -> u32 {
    let c = floor(uv);
    let variant = edge(c, 0.0, id)
        + 2u * edge(c + vec2<f32>(1.0, 0.0), 0.0, id)
        + 4u * edge(c, 1.0, id)
        + 8u * edge(c + vec2<f32>(0.0, 1.0), 1.0, id);
    return variant * 3u + k;
}

fn layer_height(k: u32) -> f32 {
    if (k == 1u) { return m.heights.x; }
    if (k == 2u) { return m.heights.y; }
    return 0.0;
}

/// What of the star reaches height `z` at `p`, past the cutout layers above.
fn starlit(p: vec2<f32>, z: f32, toward: vec3<f32>, id: f32) -> f32 {
    var through = 1.0;
    for (var k = 1u; k <= 2u; k++) {
        let zk = layer_height(k);
        if (zk > z) {
            let at = p + toward.xy * (zk - z) / toward.z;
            let cover = textureSampleLevel(tiles, tile_sampler, at / m.layer.y, variant_layer(at / m.layer.y, k, id), 1.0).a;
            through *= 1.0 - smoothstep(0.35, 0.65, cover);
        }
    }
    return through;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let f = face(in.world_normal);
    let k = u32(m.layer.x + 0.5);
    // Each face starts its tile somewhere else, so neighbors do not repeat edge to edge.
    let p = vec2<f32>(dot(in.world_position, f.u), dot(in.world_position, f.v)) + hash2(f.id) * m.layer.y;
    let uv = p / m.layer.y;
    let layer = variant_layer(uv, k, f.id);
    let texel = textureSample(tiles, tile_sampler, uv, layer);
    let baked = textureSample(light, tile_sampler, uv, layer).rgb;
    if (k > 0u && texel.a < 0.5) {
        discard;
    }
    let tn = vec2<f32>(texel.r, texel.g) * 2.0 - 1.0;
    let n = normalize(f.u * tn.x + f.v * tn.y + f.n * sqrt(max(1.0 - dot(tn, tn), 0.0)));
    let albedo = texel.b;

    var radiance = m.ambient.rgb * albedo + baked * m.lamp_color.rgb;

    let star = normalize(m.to_star.xyz);
    let toward = vec3<f32>(dot(star, f.u), dot(star, f.v), dot(star, f.n));
    if (toward.z > 0.0) {
        let to_eye = normalize(view.world_position - in.world_position);
        let half_way = normalize(star + to_eye);
        let spec = m.finish.z * pow(max(dot(n, half_way), 0.0), m.finish.y);
        let ndl = max(dot(n, star), 0.0);
        radiance += m.star_color.rgb * starlit(p, layer_height(k), toward, f.id) * ndl * (albedo + spec);
    }

    return vec4<f32>(radiance * m.finish.x, 1.0);
}
