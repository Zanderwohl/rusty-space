// Body wireframe shader with emissive bloom modulated by vertex alpha brightness.
// Used for lat/lon grid lines and terminator circles.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(5) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(5) color: vec4<f32>,
}

struct BodyWireframeMaterialUniform {
    base_color: vec4<f32>,
    emission_strength: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: BodyWireframeMaterialUniform;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);

    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    out.color = vertex.color;

    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let brightness = in.color.a;
    let emissive = material.base_color.rgb * (1.0 + brightness * material.emission_strength);
    return vec4(emissive, 1.0);
}
