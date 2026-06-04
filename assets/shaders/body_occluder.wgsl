// Body occluder shader with Lambert day/night shading.
// Night side is fully dark; day side shows a subtle color via N·L.

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
    @location(1) lambert: f32,
}

struct OccluderMaterialUniform {
    base_color: vec4<f32>,
    num_suns: u32,
    sun_dir_0: vec4<f32>,
    sun_dir_1: vec4<f32>,
    sun_dir_2: vec4<f32>,
    sun_dir_3: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: OccluderMaterialUniform;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);

    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(out.world_position.xyz);

    let n = normalize(vertex.normal);

    var lam = 0.0;
    if (material.num_suns > 0u) {
        lam += max(0.0, dot(n, material.sun_dir_0.xyz));
    }
    if (material.num_suns > 1u) {
        lam += max(0.0, dot(n, material.sun_dir_1.xyz));
    }
    if (material.num_suns > 2u) {
        lam += max(0.0, dot(n, material.sun_dir_2.xyz));
    }
    if (material.num_suns > 3u) {
        lam += max(0.0, dot(n, material.sun_dir_3.xyz));
    }
    out.lambert = lam;

    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4(material.base_color.rgb * in.lambert, 1.0);
}
