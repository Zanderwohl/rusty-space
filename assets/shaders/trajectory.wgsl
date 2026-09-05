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
    @location(2) distance_dim: f32,
    @location(5) color: vec4<f32>,
}

// Material uniforms - must match TrajectoryMaterial struct field order
struct TrajectoryMaterialUniform {
    base_color: vec4<f32>,
    brightness_threshold: f32,
    emission_strength: f32,
    base_tube_radius: f32,
    target_tube_radius: f32,
    dynamic_thickness: f32,
    front: f32,
    back: f32,
    exposure: f32,
    glow_gain: f32,
    phase_now: f32,
    phase_wrap: f32,
    distance_dim: f32,
}

// Distance (bevy metres) at/below which trajectories are at full brightness, and the
// falloff past it. These live here rather than on the CPU: baking dimming into vertices
// meant every camera move rebuilt the mesh.
const DISTANCE_DIM_REF: f32 = 5.0;
const DISTANCE_DIM_POWER: f32 = 0.4;
const DISTANCE_DIM_MIN: f32 = 0.01;

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: TrajectoryMaterialUniform;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    
    // Get the model matrix for this instance
    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);

    // Base world position for per-vertex distance-to-origin thickness scaling.
    let base_world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    let distance_from_origin = length(base_world_position.xyz);

    // Match CPU thickness curve: power-law with angular floor and hard clamps.
    let min_tube_radius = 0.000005;
    let max_tube_radius = 1000.0;
    let reference_distance = 10.0;
    let radius_scale_power = 0.75;
    let min_angular_size = 0.0005;
    let scale_factor = pow(max(distance_from_origin, 0.000001) / reference_distance, radius_scale_power);
    let dynamic_radius = clamp(
        max(material.base_tube_radius * scale_factor, min_angular_size * distance_from_origin),
        min_tube_radius,
        max_tube_radius
    );

    let desired_radius = select(
        material.target_tube_radius,
        dynamic_radius,
        material.dynamic_thickness > 0.5
    );

    // Displace vertex along tube normal to adjust thickness without mesh rebuild.
    let radius_delta = desired_radius - material.base_tube_radius;
    let displaced_position = vertex.position + vertex.normal * radius_delta;
    
    // Transform position to world space
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(displaced_position, 1.0));
    
    // Transform to clip space
    out.clip_position = position_world_to_clip(out.world_position.xyz);
    
    // Transform normal to world space
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    
    // Distance dimming, from the same range the thickness curve above already measured.
    // This used to be baked into vertex rgb on the CPU, which meant every camera move
    // rebuilt the mesh.
    out.distance_dim = select(
        1.0,
        max(pow(DISTANCE_DIM_REF / max(distance_from_origin, 0.000001), DISTANCE_DIM_POWER), DISTANCE_DIM_MIN),
        distance_from_origin > DISTANCE_DIM_REF
    );

    // Pass through vertex color (contains brightness or phase in alpha)
    out.color = vertex.color;
    
    return out;
}

const NEAR_FADE_DISTANCE: f32 = 1.0;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Vertex color carries two per-vertex factors:
    //   alpha = along-length lerp factor t (0..1), or a static phase when phase_wrap is set
    //   rgb   = per-vertex amplitude; superseded by the shader's own dimming when
    //           distance_dim is set. 1.0 = no attenuation
    // Final brightness lerps front->back by t, scaled by amplitude and exposure.
    // Markers bake a final brightness into alpha and use the identity range
    // (front=0, back=1, exposure=0), so mix(0, 1, alpha) == alpha leaves them unchanged.
    // Vertex alpha is either `t` itself or a static along-orbit phase; `phase_wrap` says
    // which. Deriving `t` per fragment is also what puts the dark->bright seam exactly at
    // the body, with no help from the geometry.
    let t = select(in.color.a, fract(in.color.a - material.phase_now), material.phase_wrap > 0.5);
    let amp = select(in.color.r, in.distance_dim, material.distance_dim > 0.5);
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
