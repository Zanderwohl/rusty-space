// A hull in regions: em_render::hull_material. lightcone/docs/32-ship-rendering.md is the look.
//
// The mesh's own position is the ship's frame in meters, so a tile is the same size in meters
// on any hull. Tiles are mipmapped and sampled with derivatives: detail below a pixel is drawn
// as its average rather than shimmering.

#import bevy_pbr::{
    mesh_functions,
    view_transformations::position_world_to_clip,
}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) region: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) ship_position: vec3<f32>,
    @location(2) ship_normal: vec3<f32>,
    // Flat, so a triangle between two parts blends one pair of materials.
    @location(3) @interpolate(flat) regions: vec2<u32>,
    @location(4) share: f32,
}

const REGIONS: u32 = 16u;

struct HullUniform {
    /// World direction to the star; `w` is the light on the unlit side.
    to_star: vec4<f32>,
    reflected: vec4<f32>,
    /// `(surface_reference, stops, 0, 0)`.
    exposure: vec4<f32>,
    /// `(tile_m, 0, 0, 0)`.
    detail: vec4<f32>,
    /// `(origin, front)`, ship frame, meters.
    reveal: vec4<f32>,
    /// `(panel_m, spread_m, 0, 0)`.
    reveal_panel: vec4<f32>,
    emitted: array<vec4<f32>, REGIONS>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: HullUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var albedo_tiles: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var tile_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var light_tiles: texture_2d_array<f32>;

/// em_render::hull_material::ALL_PLATED.
const ALL_PLATED: f32 = 1.0e30;

/// Rec. 709, matching lc-client's tonemap.
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    out.ship_position = vertex.position;
    out.ship_normal = vertex.normal;
    out.regions = vec2<u32>(u32(vertex.region.x + 0.5), u32(vertex.region.y + 0.5));
    out.share = vertex.region.z;
    return out;
}

struct Texel {
    albedo: vec3<f32>,
    lit: f32,
}

/// The three projections' weights. Sharp, so two tiles overlap only on a narrow band.
fn planes(n: vec3<f32>) -> vec3<f32> {
    let a = pow(abs(normalize(n)), vec3<f32>(8.0));
    return a / (a.x + a.y + a.z);
}

/// Region `layer`'s tile at ship position `p`, mapped triplanar. Each plane's `v` is along the
/// ship's y where it can be, so window bands run the same way on every side.
fn triplanar(p: vec3<f32>, w: vec3<f32>, layer: u32) -> Texel {
    let s = p / material.detail.x;
    let on_x = s.zy;
    let on_y = s.xz;
    let on_z = s.xy;
    let albedo = textureSample(albedo_tiles, tile_sampler, on_x, layer).rgb * w.x
        + textureSample(albedo_tiles, tile_sampler, on_y, layer).rgb * w.y
        + textureSample(albedo_tiles, tile_sampler, on_z, layer).rgb * w.z;
    let lit = textureSample(light_tiles, tile_sampler, on_x, layer).r * w.x
        + textureSample(light_tiles, tile_sampler, on_y, layer).r * w.y
        + textureSample(light_tiles, tile_sampler, on_z, layer).r * w.z;
    return Texel(albedo, lit);
}

fn hash(cell: vec3<f32>) -> f32 {
    let q = fract(cell * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yxz + 33.33);
    return fract((r.x + r.y) * r.z);
}

/// Whether the plating has reached the panel at `p`. Panels are cells of the dominant
/// projection, each with its own hashed lag, so the front arrives as panels, not as a line.
fn plated(p: vec3<f32>, n: vec3<f32>) -> bool {
    let front = material.reveal.w;
    if (front >= ALL_PLATED) {
        return true;
    }
    let panel_m = material.reveal_panel.x;
    let a = abs(n);
    // The dominant axis is left alone, so the cell is a column through the hull and its
    // center stays on the surface.
    var keep = vec3<f32>(0.0);
    if (a.x >= a.y && a.x >= a.z) {
        keep.x = 1.0;
    } else if (a.y >= a.z) {
        keep.y = 1.0;
    } else {
        keep.z = 1.0;
    }
    let cell = floor(p / panel_m) * (1.0 - keep) + keep * 0.5;
    let center = mix((cell + 0.5) * panel_m, p, keep);
    let lag = hash(cell + keep * 17.0) * material.reveal_panel.y;
    return distance(center, material.reveal.xyz) + lag <= front;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let w = planes(in.ship_normal);
    let near = triplanar(in.ship_position, w, in.regions.x);
    let far = triplanar(in.ship_position, w, in.regions.y);
    if (!plated(in.ship_position, in.ship_normal)) {
        discard;
    }
    let t = saturate(in.share);
    let albedo = mix(near.albedo, far.albedo, t);
    let emitted = mix(
        near.lit * material.emitted[min(in.regions.x, REGIONS - 1u)].rgb,
        far.lit * material.emitted[min(in.regions.y, REGIONS - 1u)].rgb,
        t,
    );

    let to_star = normalize(material.to_star.xyz);
    let lambert = max(dot(normalize(in.world_normal), to_star), 0.0);
    let light = max(lambert, material.to_star.w);
    let linear = material.reflected.rgb * albedo * light + emitted;

    let reference = material.exposure.x;
    let stops = material.exposure.y;
    let luminance = dot(linear, LUMA);
    let peak = max(linear.r, max(linear.g, linear.b));
    var value = 0.0;
    if (luminance > 0.0 && reference > 0.0 && stops > 0.0) {
        value = clamp(log2(luminance / reference) / stops + 1.0, 0.0, 1.0);
    }
    let chroma = select(vec3<f32>(1.0), linear / peak, peak > 0.0);
    return vec4<f32>(chroma * value, 1.0);
}
