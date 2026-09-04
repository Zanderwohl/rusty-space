// Distant body point shader with emissive bloom.
// Brightness is pre-computed on CPU from phase angles to all stars.

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
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
}

struct BodyPointMaterialUniform {
    base_color: vec4<f32>,
    brightness: f32,
    emission_strength: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: BodyPointMaterialUniform;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);

    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);

    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let view_dir = normalize(view.world_position.xyz - in.world_position.xyz);
    let normal = normalize(in.world_normal);
    let NdotV = max(dot(normal, view_dir), 0.0);

    // Tight bright core + soft wide halo
    let core = pow(NdotV, 6.0);
    let halo = pow(NdotV, 1.5);
    let glow = core * 0.6 + halo * 0.4;

    let emissive = material.base_color.rgb * material.brightness * material.emission_strength * glow;
    return vec4(emissive, glow);
}
