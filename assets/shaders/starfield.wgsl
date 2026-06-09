// Starfield background shader.
// Renders stars as dots on a sky sphere based on pre-computed direction vectors.
// Stars are stored as vec4(dir.x, dir.y, dir.z, mag) in Bevy Y-up space.

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
    @location(1) local_position: vec3<f32>,
}

struct StarfieldMaterialUniform {
    star_count: u32,
    brightness: f32,
    _padding: vec2<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<storage, read> stars: array<vec4<f32>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<storage, read> colors: array<vec4<f32>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> material: StarfieldMaterialUniform;

// Dot product threshold for star visibility (controls apparent star size)
// 0.999995 ≈ 0.18 degrees, roughly 2-3 pixels radius
const STAR_THRESHOLD: f32 = 0.999995;

// Brightness scaling: maps magnitude to HDR output
// Sirius (mag -1.5) -> ~2.0, naked eye limit (mag 6) -> ~0.05
const BRIGHTNESS_SCALE: f32 = 0.15;
const MAG_ZERO_BRIGHTNESS: f32 = 1.0;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);

    let world_pos = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    out.world_position = world_pos.xyz;
    out.clip_position = position_world_to_clip(world_pos.xyz);

    // Clamp depth to far plane so starfield renders behind everything
    out.clip_position.z = out.clip_position.w;

    // Pass local position for direction calculation (sphere is centered at origin)
    out.local_position = vertex.position;

    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // View direction from camera toward this fragment on the sphere
    let view_dir = normalize(in.local_position);

    var total_color: vec3<f32> = vec3<f32>(0.0, 0.0, 0.0);

    // Loop through all stars and accumulate colored brightness
    for (var i: u32 = 0u; i < material.star_count; i = i + 1u) {
        let star = stars[i];
        let star_dir = star.xyz;
        let mag = star.w;

        // Angular proximity: dot product of 1.0 means perfect alignment
        let alignment = dot(view_dir, star_dir);

        if alignment > STAR_THRESHOLD {
            // Convert magnitude to brightness (lower mag = brighter)
            let brightness = MAG_ZERO_BRIGHTNESS * pow(2.512, -mag) * BRIGHTNESS_SCALE;

            // Smooth falloff from center to reduce flickering/aliasing
            let falloff = (alignment - STAR_THRESHOLD) / (1.0 - STAR_THRESHOLD);

            let star_color = colors[i].rgb;
            total_color += star_color * brightness * falloff;
        }
    }

    let emission_strength = material.brightness;
    let emissive = total_color * emission_strength;
    return vec4(emissive, 1.0);
}
