// Trajectory tube shader with emissive bloom for bright segments and alpha fade for dim segments.
// Brightness is encoded in vertex color alpha by the CPU.
// Near-fade is applied per-fragment for smooth fade at close range.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

// Vertex input - matches Bevy's standard mesh vertex layout
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(5) color: vec4<f32>,
}

// Vertex output / Fragment input
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(5) color: vec4<f32>,
}

// Material uniforms - must match TrajectoryMaterial struct layout
struct TrajectoryMaterialUniform {
    base_color: vec4<f32>,
    brightness_threshold: f32,
    emission_strength: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: TrajectoryMaterialUniform;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    
    // Get the model matrix for this instance
    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    
    // Transform position to world space
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    
    // Transform to clip space
    out.clip_position = position_world_to_clip(out.world_position.xyz);
    
    // Transform normal to world space
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    
    // Pass through vertex color (contains brightness in alpha)
    out.color = vertex.color;
    
    return out;
}

const NEAR_FADE_DISTANCE: f32 = 1.0;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Brightness is encoded in vertex color alpha
    var brightness = in.color.a;
    
    // Per-fragment near-fade: smooth fade by distance² below 1 bevy meter
    let dist_to_camera = length(in.world_position.xyz - view.world_position.xyz);
    if dist_to_camera < NEAR_FADE_DISTANCE {
        let near_fade = (dist_to_camera / NEAR_FADE_DISTANCE) * (dist_to_camera / NEAR_FADE_DISTANCE);
        brightness = brightness * near_fade;
    }
    
    // Combine base material color with vertex color RGB
    let base = material.base_color.rgb * in.color.rgb;
    
    // Above threshold: emit HDR values that trigger Bloom
    // Below threshold: fade alpha toward 0 for transparency
    if brightness > material.brightness_threshold {
        // Normalize brightness above threshold to 0..1 range for emission scaling
        let normalized = (brightness - material.brightness_threshold) / (1.0 - material.brightness_threshold);
        let emission = base * (1.0 + normalized * material.emission_strength);
        return vec4(emission, 1.0);
    } else {
        // Fade alpha based on how far below threshold
        let alpha = brightness / material.brightness_threshold;
        return vec4(base, alpha);
    }
}
