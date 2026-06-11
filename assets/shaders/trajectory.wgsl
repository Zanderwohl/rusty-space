// Trajectory tube shader with emissive bloom for bright segments and alpha fade for dim segments.
// Vertex alpha carries the along-length lerp factor t (0..1) and vertex rgb a per-vertex
// amplitude; the front/back/exposure uniforms turn these into the final brightness, so
// brightness can change live without rebuilding the mesh.
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

// Material uniforms - must match TrajectoryMaterial struct field order
struct TrajectoryMaterialUniform {
    base_color: vec4<f32>,
    brightness_threshold: f32,
    emission_strength: f32,
    front: f32,
    back: f32,
    exposure: f32,
    glow_gain: f32,
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
    // Vertex color carries two per-vertex factors:
    //   alpha = along-length lerp factor t (0..1)
    //   rgb   = per-vertex amplitude (e.g. distance dimming); 1.0 = no attenuation
    // Final brightness lerps front->back by t, scaled by amplitude and exposure.
    // Markers bake a final brightness into alpha and use the identity range
    // (front=0, back=1, exposure=0), so mix(0, 1, alpha) == alpha leaves them unchanged.
    let t = in.color.a;
    let amp = in.color.r;
    var brightness = mix(material.front, material.back, t) * amp * pow(2.0, -material.exposure) * material.glow_gain;

    // Per-fragment near-fade: smooth fade by distance² below 1 bevy meter
    let dist_to_camera = length(in.world_position.xyz - view.world_position.xyz);
    if dist_to_camera < NEAR_FADE_DISTANCE {
        let near_fade = (dist_to_camera / NEAR_FADE_DISTANCE) * (dist_to_camera / NEAR_FADE_DISTANCE);
        brightness = brightness * near_fade;
    }

    // Base hue comes from the material; rgb is repurposed as amplitude above.
    let base = material.base_color.rgb;
    
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
