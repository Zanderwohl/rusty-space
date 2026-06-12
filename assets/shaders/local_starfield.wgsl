// Local-star skybox pass shader.
//
// Each simulation star is rendered as an emissive billboard using:
//   POSITION : star relative position (world vector from camera to star)
//   COLOR    : star linear RGB color
//   CORNER   : quad corner in [-1, 1]^2
//   PARAMS   : (intensity, radius, _)
//
// The vertex stage computes angular size from physical radius / distance,
// clamped to a minimum angular footprint to reduce flicker.

#import bevy_pbr::mesh_view_bindings::view

struct Vertex {
    @location(0) relative_position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) corner: vec2<f32>,
    @location(3) params: vec3<f32>, // intensity, radius, unused
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) color: vec3<f32>,
    @location(2) intensity: f32,
    @location(3) distance: f32,
}

struct LocalStarfieldMaterialUniform {
    brightness_min: f32,
    brightness_max: f32,
    min_angular_radius_rad: f32,
    max_intensity: f32,
    near_distance_bevy: f32,
    far_distance_bevy: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: LocalStarfieldMaterialUniform;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    let rel = vertex.relative_position;
    let distance = max(length(rel), 1e-6);
    let dir_world = rel / distance;
    let dir_view = normalize((view.view_from_world * vec4<f32>(dir_world, 0.0)).xyz);

    let radius = max(vertex.params.y, 0.0);
    let angular_radius = max(radius / distance, material.min_angular_radius_rad);

    let pos_view = dir_view + vec3<f32>(vertex.corner, 0.0) * angular_radius;
    var clip = view.clip_from_view * vec4<f32>(pos_view, 1.0);
    clip.z = clip.w;

    out.clip_position = clip;
    out.corner = vertex.corner;
    out.color = vertex.color.rgb;
    out.intensity = max(vertex.params.x, 0.0);
    out.distance = distance;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let r = length(in.corner);
    if (r > 1.0) {
        discard;
    }
    let radial_falloff = 1.0 - r;

    var norm_intensity: f32 = 0.0;
    if (material.max_intensity > 0.0) {
        norm_intensity = in.intensity / material.max_intensity;
    }

    let near_d = max(material.near_distance_bevy, 1e-6);
    let far_d = max(material.far_distance_bevy, near_d);
    let dist = clamp(in.distance, near_d, far_d);

    // Normalize inverse-square between 1m and 1ly (converted to Bevy units).
    let inv_near = 1.0 / (near_d * near_d);
    let inv_far = 1.0 / (far_d * far_d);
    let inv_d = 1.0 / (dist * dist);
    let inv_t = clamp((inv_d - inv_far) / max(inv_near - inv_far, 1e-12), 0.0, 1.0);

    let distance_brightness = mix(material.brightness_max, material.brightness_min, inv_t);
    let brightness = distance_brightness * norm_intensity * radial_falloff;
    let emissive = in.color * brightness;
    return vec4<f32>(emissive, 1.0);
}
