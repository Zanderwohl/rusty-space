// A rocky world's air against space: the limb, on a shell em_render::atmosphere_material
// places around the body. The disc's own air is scattered by body_surface.wgsl, and the body
// hides this shell's back faces there.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}
#import lightcone::scatter::{air_of, crossing, scatter, top_of}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) center: vec3<f32>,
    /// The shell's radius, world units.
    @location(2) top: f32,
}

struct AtmosphereUniform {
    to_star: vec4<f32>,
    starlight: vec4<f32>,
    exposure: vec4<f32>,
    gas: vec4<f32>,
    haze: vec4<f32>,
    albedo: vec4<f32>,
    glow: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: AtmosphereUniform;

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_position = world.xyz;
    out.center = world_from_local[3].xyz;
    out.top = length(world_from_local[0].xyz);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let air = air_of(material.gas, material.haze, material.albedo);
    let radius = in.top / top_of(air);
    let o = (view.world_position - in.center) / radius;
    let d = normalize(in.world_position - view.world_position);
    let shell = crossing(o, d, top_of(air));
    let body = crossing(o, d, 1.0);
    // Up to the ground where the ray finds it: the mesh is inscribed in the sphere, so a sliver
    // of the true disc falls outside it and is this shell's to draw.
    var t1 = shell.y;
    if (body.x < body.y && body.x > 0.0) {
        t1 = body.x;
    }
    let s = scatter(o, d, max(shell.x, 0.0), t1, normalize(material.to_star.xyz), air);
    // What the air takes at ten microns it gives back at its own temperature: the limb glows.
    let linear = s.light * material.starlight.rgb + material.glow.rgb * (1.0 - exp(-air.infrared * s.column));

    let reference = material.exposure.x;
    let stops = material.exposure.y;
    let luminance = dot(linear, LUMA);
    let peak = max(linear.r, max(linear.g, linear.b));
    var value = 0.0;
    if (luminance > 0.0 && reference > 0.0 && stops > 0.0) {
        value = clamp(log2(luminance / reference) / stops + 1.0, 0.0, 1.0);
    }
    let chroma = select(vec3<f32>(1.0), linear / peak, peak > 0.0);
    // Alpha zero: pure addition over what is behind.
    return vec4<f32>(chroma * value, 0.0);
}
