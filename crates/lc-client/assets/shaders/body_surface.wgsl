// A resolved body's surface.
//
// The pattern is a texture graph baked onto a cubemap -- see lc-client's surfaces module for
// which graph -- and the palette is the body's class's, from lc_world::surface. A body with a
// graph of its own has a color cubemap instead, and may have a cloud deck. Every cubemap is
// sampled on the body-fixed direction, which is what the mesh's local position already is, so
// the surface turns with the body and does not swim with the camera.

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
    @location(0) world_normal: vec3<f32>,
    @location(1) local_direction: vec3<f32>,
}

struct BodySurfaceUniform {
    /// The two ends of the surface's color, linear.
    dark: vec4<f32>,
    light: vec4<f32>,
    /// World direction to the star. `w` is the ambient floor on the night side.
    to_star: vec4<f32>,
    /// `(color, contrast, clouds, unused)`: whether the color and cloud cubemaps are drawn.
    params: vec4<f32>,
    /// Starlight the surface reflects, as linear display light before the tone map.
    reflected: vec4<f32>,
    /// Light the body makes itself, in the same units. `w` is how far the pattern inverts in it.
    emitted: vec4<f32>,
    /// `(surface_reference, stops, 0, 0)`.
    exposure: vec4<f32>,
}

/// Rec. 709, matching crate::tonemap.
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: BodySurfaceUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var pattern: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var pattern_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var color: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var clouds: texture_cube<f32>;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_normal = normalize(mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index));
    // The mesh's own position is the body-fixed direction, because it is a unit sphere. So the
    // pattern turns with the body and is a function of nothing else.
    out.local_direction = normalize(vertex.position);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let contrast = material.params.y;
    let surface = textureSample(pattern, pattern_sampler, in.local_direction).r;
    let t = mix(0.5, surface, contrast);
    var albedo = mix(material.dark.rgb, material.light.rgb, t);
    let own = textureSample(color, pattern_sampler, in.local_direction).rgb;
    let deck = textureSample(clouds, pattern_sampler, in.local_direction);
    albedo = mix(albedo, own, material.params.x);
    albedo = mix(albedo, deck.rgb, deck.a * material.params.z);

    // Lambert, with a soft terminator. A hard one is a straight line across the disc and reads
    // as a cut rather than as a horizon.
    let lambert = dot(normalize(in.world_normal), normalize(material.to_star.xyz));
    let lit = smoothstep(-0.12, 0.25, lambert);
    let light = max(lit, material.to_star.w);

    // The body's own light, which does not care where the star is. In the optical it is zero
    // and this is the shader it always was; at ten microns it is the whole picture, and a
    // giant's night side stops being dark because nothing was lighting it in the first place.
    //
    // The pattern inverts: a belt is a gap in the cloud deck, so it reflects less and lets more
    // of the warm interior out. Mean-preserving about one, so the band the body is seen in
    // moves its pattern about rather than changing how much light it sends.
    let inversion = material.emitted.w;
    let linear = material.reflected.rgb * albedo * light
        + material.emitted.rgb * mix(1.0 + inversion, 1.0 - inversion, t);

    // The tone map of crate::tonemap, evaluated here rather than per body: the two terms mix
    // differently across the disc and the curve is logarithmic, so one level for the whole
    // surface gets the terminator wrong in any band where both terms matter.
    let reference = material.exposure.x;
    let stops = material.exposure.y;
    let luminance = dot(linear, LUMA);
    let peak = max(linear.r, max(linear.g, linear.b));
    var value = 0.0;
    if (luminance > 0.0 && reference > 0.0 && stops > 0.0) {
        value = clamp(log2(luminance / reference) / stops + 1.0, 0.0, 1.0);
    }
    let chroma = select(vec3<f32>(1.0), linear / peak, peak > 0.0);

    return vec4<f32>(chroma * value, 1.0);
}
