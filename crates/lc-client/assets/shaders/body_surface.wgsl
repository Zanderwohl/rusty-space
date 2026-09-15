// A resolved body's surface, generated rather than stored.
//
// No textures and no authored appearance. What a body looks like follows from what it is --
// see lc_world::surface -- and everything past that is a seed. Two families cover the solar
// system: latitude bands for anything gaseous, and mottling for everything solid.
//
// The noise is sampled on the body-fixed direction, which is what the mesh's local position
// already is, so the pattern turns with the body and does not swim with the camera.

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
    /// The two ends of the surface's colour, linear.
    dark: vec4<f32>,
    light: vec4<f32>,
    /// World direction to the star. `w` is the ambient floor on the night side.
    to_star: vec4<f32>,
    /// `(unused, contrast, seed, banded)`.
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

fn hash31(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn value_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let x00 = mix(hash31(i), hash31(i + vec3<f32>(1.0, 0.0, 0.0)), w.x);
    let x10 = mix(hash31(i + vec3<f32>(0.0, 1.0, 0.0)), hash31(i + vec3<f32>(1.0, 1.0, 0.0)), w.x);
    let x01 = mix(hash31(i + vec3<f32>(0.0, 0.0, 1.0)), hash31(i + vec3<f32>(1.0, 0.0, 1.0)), w.x);
    let x11 = mix(hash31(i + vec3<f32>(0.0, 1.0, 1.0)), hash31(i + vec3<f32>(1.0, 1.0, 1.0)), w.x);
    return mix(mix(x00, x10, w.y), mix(x01, x11, w.y), w.z);
}

fn fbm(p: vec3<f32>, octaves: u32) -> f32 {
    var sum = 0.0;
    var amplitude = 0.5;
    var frequency = 1.0;
    var total = 0.0;
    for (var i = 0u; i < octaves; i = i + 1u) {
        sum = sum + amplitude * value_noise(p * frequency);
        total = total + amplitude;
        frequency = frequency * 2.13;
        amplitude = amplitude * 0.5;
    }
    return sum / max(total, 1e-6);
}

/// Where on the palette this point of the surface sits, `[0, 1]`.
fn surface(direction: vec3<f32>, seed: f32, banded: bool) -> f32 {
    let offset = vec3<f32>(seed, seed * 1.7, seed * 2.9);
    if (banded) {
        // Bands in latitude, warped by turbulence so they are not stripes. The warp has to stay
        // well under the band spacing: at a quarter of a period it stops perturbing the bands
        // and starts destroying them, and the surface reads as blobs.
        let spacing = 18.0;
        let turbulence = fbm(direction * 3.1 + offset, 4u) - 0.5;
        let drift = (fbm(direction * 1.1 + offset, 2u) - 0.5) * 0.9;
        let bands = sin((direction.y + turbulence * 0.055) * spacing + drift);
        // Sharpened, because a fluid's bands have edges rather than a sinusoid's gradient.
        return clamp(0.5 + 0.62 * bands, 0.0, 1.0);
    }
    // Solid: mottling, with a sharper high-frequency term for the look of relief.
    let broad = fbm(direction * 2.2 + offset, 5u);
    let fine = fbm(direction * 9.0 + offset, 3u);
    return clamp(broad * 0.75 + fine * 0.25, 0.0, 1.0);
}

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
    let seed = material.params.z;
    let banded = material.params.w > 0.5;

    let t = mix(0.5, surface(in.local_direction, seed, banded), contrast);
    let albedo = mix(material.dark.rgb, material.light.rgb, t);

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
