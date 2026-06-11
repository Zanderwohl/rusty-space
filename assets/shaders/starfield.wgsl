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
    star_radius_min: f32,
    star_radius_max: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<storage, read> stars: array<vec4<f32>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<storage, read> colors: array<vec4<f32>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> material: StarfieldMaterialUniform;

// Arcminutes -> radians (pi / (180 * 60)). The star radius uniforms are in arcmin;
// the visibility test is a dot product, so radius is converted to a cosine threshold.
const ARCMIN_TO_RAD: f32 = 0.0002908882;

// Magnitude range mapped onto the [star_radius_min, star_radius_max] size range.
// Brighter (lower magnitude) stars are drawn larger.
const MAG_BRIGHT: f32 = -1.5; // ~Sirius -> max radius
const MAG_FAINT: f32 = 6.0;   // naked-eye limit -> min radius

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

        // Per-star angular radius: brighter stars (lower mag) are drawn larger.
        let size_t = clamp((MAG_FAINT - mag) / (MAG_FAINT - MAG_BRIGHT), 0.0, 1.0);
        let radius_arcmin = mix(material.star_radius_min, material.star_radius_max, size_t);
        let threshold = cos(radius_arcmin * ARCMIN_TO_RAD);

        if alignment > threshold {
            // Convert magnitude to brightness (lower mag = brighter)
            let brightness = MAG_ZERO_BRIGHTNESS * pow(2.512, -mag) * BRIGHTNESS_SCALE;

            // Smooth falloff from center to reduce flickering/aliasing
            let falloff = (alignment - threshold) / (1.0 - threshold);

            let star_color = colors[i].rgb;
            total_color += star_color * brightness * falloff;
        }
    }

    let emission_strength = material.brightness;
    let emissive = total_color * emission_strength;
    return vec4(emissive, 1.0);
}
