// A drive's exhaust: a volume of glowing gas, integrated along the view ray.
//
// The mesh is a proxy and its shape never shows. What is drawn is a cone of gas inside it,
// widening from the nozzle and thinning as it goes, and every fragment works out how much of
// that cone its own ray passes through. That is where the feathered edge comes from: a ray
// grazing the side crosses almost nothing, one down the middle crosses the lot.
//
// Local space is the proxy's. The axis is `+y`, running from `-0.5` at the nozzle to `+0.5` at
// the far end, and a radius of one is the proxy wall.

#import bevy_pbr::{
    mesh_functions,
    view_transformations::position_world_to_clip,
}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec3<f32>,
}

struct PlumeUniform {
    glow: vec4<f32>,
    /// `(throat, mouth, edge, taper)`.
    shape: vec4<f32>,
    eye_local: vec4<f32>,
    /// `(surface_reference, stops, brightness, unused)`.
    exposure: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: PlumeUniform;

const STEPS: i32 = 24;
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    out.clip_position = position_world_to_clip(world.xyz);
    out.local = vertex.position;
    return out;
}

/// How much gas is at a point of the proxy, in arbitrary units.
///
/// Zero outside the length, and a smooth falloff to the side rather than a wall — a plume in
/// vacuum has no boundary, it just runs out of gas.
fn density(p: vec3<f32>) -> f32 {
    let along = p.y + 0.5;
    if (along < 0.0 || along > 1.0) {
        return 0.0;
    }
    // The cone: from the nozzle out to the mouth. This is the expansion ratio, drawn.
    let radius = mix(material.shape.x, material.shape.y, along);
    let across = length(p.xz) / max(radius, 1e-6);
    // Gaussian across, so the edge feathers instead of cutting.
    let profile = exp(-across * across * material.shape.z);
    // Thinning along, as it spreads and cools.
    //
    // No `1/r^2` on top of it: that was a second go at the same idea, and between them the
    // column came out a hundred times deeper at the nozzle than at the mouth, which put every
    // part of the plume above the top of the exposure window and turned the whole cone into one
    // flat saturated shape. Bounded by one here, so the depth a ray accumulates is of order the
    // distance it travelled and the brightness scale below means something.
    let fading = pow(max(1.0 - along, 0.0), material.shape.w);
    return profile * fading;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // The ray, in the proxy's own space. Back faces are what is drawn, so this fragment is on
    // the wall *behind* the gas and the march runs from here back toward the eye.
    //
    // **Backwards, and that is not a preference.** A plume is metres long and sits an
    // astronomical unit from the render origin, so the eye is of order `1e8` in the proxy's own
    // units — and `eye + direction * t` then asks `f32` for a point near the origin as the
    // difference of two numbers near `1e8`, where its spacing is about eight. Every position
    // the density is sampled at comes out quantised to nothing and the plume does not appear at
    // all. Starting from the fragment keeps every term of order one.
    let eye = material.eye_local.xyz;
    let toward = in.local - eye;
    let range = length(toward);
    if (range <= 0.0) {
        return vec4<f32>(0.0);
    }
    let direction = toward / range;

    // How far back the ray stays inside the slab the gas lives in. A ray along the axis never
    // leaves it, and is bounded by the proxy's own width instead.
    var span = 2.0;
    if (abs(direction.y) > 1e-6) {
        let leaves = select(in.local.y - 0.5, in.local.y + 0.5, direction.y > 0.0);
        span = abs(leaves / direction.y);
    }
    // And never further than the eye itself, for when the camera is inside the proxy.
    span = min(span, range);
    if (span <= 0.0) {
        return vec4<f32>(0.0);
    }

    // Emission only, with no absorption: the gas is thin and it is the brightest thing in the
    // frame, so what reaches the eye is the sum of what every part of it puts out.
    let step = span / f32(STEPS);
    var depth = 0.0;
    for (var i = 0; i < STEPS; i = i + 1) {
        let back = (f32(i) + 0.5) * step;
        depth = depth + density(in.local - direction * back) * step;
    }
    if (depth <= 0.0) {
        return vec4<f32>(0.0);
    }

    let linear = material.glow.rgb * depth * material.exposure.z;

    // The tone map of crate::tonemap, the same curve the lit surfaces evaluate, so a plume and
    // the planet it is flying past sit in one exposure rather than two that agree.
    let reference = material.exposure.x;
    let stops = material.exposure.y;
    let luminance = dot(linear, LUMA);
    let peak = max(linear.r, max(linear.g, linear.b));
    var value = 0.0;
    if (luminance > 0.0 && reference > 0.0 && stops > 0.0) {
        value = clamp(log2(luminance / reference) / stops + 1.0, 0.0, 1.0);
    }
    let chroma = select(vec3<f32>(1.0), linear / peak, peak > 0.0);

    // **Alpha zero.** `AlphaMode::Add` is premultiplied — `src + dst * (1 - alpha)` — so
    // anything else here would rub out what is behind the plume instead of adding to it.
    return vec4<f32>(chroma * value, 0.0);
}
