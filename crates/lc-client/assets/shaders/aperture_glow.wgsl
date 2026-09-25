// An engine's open face, and a short glow off it along the exhaust.
//
// The face is a disk of radius one at `y = 0` facing `+y`, radiating what the host says, which
// is a blackbody far past the top of any exposure. The glow is a Gaussian ellipsoid centered half
// its reach aft of the face and cut off at it, integrated along the ray in closed form.
//
// Local space is the proxy's, in aperture radii.

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

struct ApertureGlowUniform {
    face: vec4<f32>,
    glow: vec4<f32>,
    /// `(reach, width, _, _)`.
    shape: vec4<f32>,
    eye_local: vec4<f32>,
    /// `(surface_reference, stops, overflow, _)`.
    exposure: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: ApertureGlowUniform;

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

/// Abramowitz and Stegun 7.1.26: within `1.5e-7`, which is below anything eight bits can show.
fn erf(x: f32) -> f32 {
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741
        + t * (-1.453152027 + t * 1.061405429))));
    return sign(x) * (1.0 - poly * exp(-a * a));
}

/// The tone map of `plume.wgsl`, which is `crate::tonemap`'s, with the overflow per channel.
fn expose(linear: vec3<f32>) -> vec3<f32> {
    let reference = material.exposure.x;
    let stops = material.exposure.y;
    let luminance = dot(linear, LUMA);
    let peak = max(linear.r, max(linear.g, linear.b));
    var above = -1e9;
    if (luminance > 0.0 && reference > 0.0 && stops > 0.0) {
        above = log2(luminance / reference);
    }
    let chroma = select(vec3<f32>(1.0), linear / peak, peak > 0.0);
    let level = clamp(above / max(stops, 1e-6) + 1.0, 0.0, 1.0);
    let over = clamp(
        log2(max(linear, vec3<f32>(1e-30)) / max(reference, 1e-30)),
        vec3<f32>(0.0),
        vec3<f32>(12.0),
    );
    return chroma * level + over * material.exposure.z;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Back from the fragment toward the eye, which may be a light-second away in these units.
    let eye = material.eye_local.xyz;
    let toward = in.local - eye;
    let range = length(toward);
    if (range <= 0.0) {
        return vec4<f32>(0.0);
    }
    let d = toward / range;
    let f = in.local;

    var linear = vec3<f32>(0.0);

    // The face, seen only from the exhaust side. Edge-on it is nothing, and `d.y` near zero is
    // exactly where the plane test would divide by it.
    if (d.y < -1e-6) {
        let t = f.y / d.y;
        let hit = f - d * t;
        if (t >= -1e-4 && t <= range && dot(hit.xz, hit.xz) <= 1.0) {
            linear += material.face.rgb;
        }
    }

    // The glow: `∫ exp(-|S (p - c)|²) dt` over the part of the ray aft of the face, with `S`
    // taking the ellipsoid to a unit sphere. A quadratic in `t`, so one `erf` difference.
    let reach = material.shape.x;
    let width = material.shape.y;
    let half = 0.5 * reach;
    let stretch = vec3<f32>(1.0 / width, 1.0 / half, 1.0 / width);
    let o = (f - vec3<f32>(0.0, half, 0.0)) * stretch;
    let v = -d * stretch;
    // Never small: `v` is a unit vector scaled by at least `1 / max(width, half)`.
    let vv = dot(v, v);
    let speed = sqrt(vv);
    let t_near = -dot(o, v) / vv;
    // The miss distance as a cross product rather than `|o|² - (o·v)²/|v|²`, which cancels
    // for a ray passing close to the center from far off.
    let miss = dot(cross(o, v), cross(o, v)) / vv;

    var t0 = 0.0;
    var t1 = range;
    if (abs(d.y) > 1e-7) {
        // `y(t) = f.y - t d.y` crosses the face at `f.y / d.y`.
        let at_face = f.y / d.y;
        if (d.y > 0.0) {
            t1 = min(t1, at_face);
        } else {
            t0 = max(t0, at_face);
        }
    } else if (f.y < 0.0) {
        t1 = t0;
    }
    if (t1 > t0) {
        // Without the `√π` of the whole integral, so side-on through the middle is `width`.
        let column = exp(-miss) * 0.5 * (erf(speed * (t1 - t_near)) - erf(speed * (t0 - t_near)))
            / speed;
        linear += material.glow.rgb * column / width;
    }

    if (dot(linear, linear) <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(expose(linear), 0.0);
}
