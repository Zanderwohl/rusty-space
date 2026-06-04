// Body wireframe shader with emissive bloom modulated by vertex alpha brightness.
// Sun directions (body-local space) provide day/night shading:
// night side = 80% base, each illuminating sun adds 20%.

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
    @location(2) sun_factor: f32,
    @location(5) color: vec4<f32>,
}

struct BodyWireframeMaterialUniform {
    base_color: vec4<f32>,
    emission_strength: f32,
    num_suns: u32,
    sun_dir_0: vec4<f32>,
    sun_dir_1: vec4<f32>,
    sun_dir_2: vec4<f32>,
    sun_dir_3: vec4<f32>,
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

    // Approximate sphere surface normal from vertex position in local space
    let sphere_normal = normalize(vertex.position);

    var sf = 1.0;
    if (material.num_suns > 0u) {
        sf = 0.5;
        if (dot(sphere_normal, material.sun_dir_0.xyz) > 0.0) { sf += 0.5; }
        if (material.num_suns > 1u && dot(sphere_normal, material.sun_dir_1.xyz) > 0.0) { sf += 0.5; }
        if (material.num_suns > 2u && dot(sphere_normal, material.sun_dir_2.xyz) > 0.0) { sf += 0.5; }
        if (material.num_suns > 3u && dot(sphere_normal, material.sun_dir_3.xyz) > 0.0) { sf += 0.5; }
    }
    out.sun_factor = sf;

    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let brightness = in.color.a;
    let emissive = material.base_color.rgb * (1.0 + brightness * material.emission_strength);
    return vec4(emissive * in.sun_factor, 1.0);
}
