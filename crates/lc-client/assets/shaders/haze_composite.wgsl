// The half-resolution haze, added into the sky pass. See `haze.rs`.

#import bevy_pbr::mesh_view_bindings::view

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var haze: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var haze_sampler: sampler;

struct Vertex {
    @location(0) position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    // At the stars' depth, under anything real: reversed depth is near / z, so the faintest
    // body writes of order 1e-15. See `starfield.wgsl`.
    out.clip_position = vec4<f32>(vertex.position.xy, 1.0e-20, 1.0);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // The sky's viewport is the corner square while the map is the view.
    let uv = (in.clip_position.xy - view.viewport.xy) / view.viewport.zw;
    let rgb = textureSampleLevel(haze, haze_sampler, uv, 0.0).rgb;
    // Additive is premultiplied: alpha zero, or it overwrites what is under it.
    return vec4<f32>(rgb, 0.0);
}
