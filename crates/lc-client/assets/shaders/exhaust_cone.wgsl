// An emission's cone as an indicator: where its heat lands, and how much.
//
// A top-hat from the apex: flux `P / (Ω d²)` inside the half-angle and nothing outside, the cone
// `lc_world::emit` models. Each fragment reads the flux at one point of its ray, the point that
// comes angularly closest to the axis as seen from the apex. From beside, that is the ray's
// closest approach to the axis; from behind or inside it still lies in the cone, where a nearest
// point to the axis line can fall behind the apex and leave a hole.
//
// Local space is the proxy's: apex at the origin, axis `+y`, the drawn length one.

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

struct ExhaustConeUniform {
    /// `(power_w, half_angle_rad, length_m, hot_w_m2)`.
    emission: vec4<f32>,
    faint: vec4<f32>,
    hot: vec4<f32>,
    eye_local: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: ExhaustConeUniform;

const PI: f32 = 3.14159265;

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

/// `(tan of the angle off the axis) / tan(half-angle)`, squared, at `p`: below one is inside.
fn depth_sq(p: vec3<f32>, tan_half: f32) -> f32 {
    if (p.y <= 0.0) {
        return 1e30;
    }
    let off = dot(p.xz, p.xz);
    return off / (p.y * p.y * tan_half * tan_half);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Back from the fragment toward the eye, as in `plume.wgsl`: the fragment is order one in
    // this space wherever the camera is, and the eye need not be.
    let eye = material.eye_local.xyz;
    let toward = in.local - eye;
    let range = length(toward);
    if (range <= 0.0) {
        return vec4<f32>(0.0);
    }
    let d = toward / range;
    let f = in.local;

    // The part of the ray inside the slab `0 <= y <= 1`, as `t` back from the fragment.
    var t0 = 0.0;
    var t1 = range;
    if (abs(d.y) > 1e-7) {
        let at_apex = f.y / d.y;
        let at_end = (f.y - 1.0) / d.y;
        t0 = max(t0, min(at_apex, at_end));
        t1 = min(t1, max(at_apex, at_end));
    }
    if (t1 < t0) {
        return vec4<f32>(0.0);
    }

    // The angle off the axis along the ray goes as `|q + t v| / (h + t w)`, which has one
    // minimum on the part with `y > 0` and it is linear: `t = -q·m / v·m` with `m = v h - q w`.
    //
    // **`v·m` cancels when the ray runs along the axis**, which is the view from inside and from
    // behind. There the minimum is far off the segment and its sign is noise. So it is only a
    // candidate: the segment's two ends are always tried too, and since the angle is unimodal
    // on the segment the smallest of the three is the minimum however wrong the third is.
    let q = f.xz;
    let v = -d.xz;
    let h = f.y;
    let w = -d.y;
    let m = v * h - q * w;
    let vm = dot(v, m);
    let tan_half = tan(material.emission.y);

    var best_t = t0;
    var best = depth_sq(f - d * t0, tan_half);
    let far = depth_sq(f - d * t1, tan_half);
    if (far < best) {
        best = far;
        best_t = t1;
    }
    if (abs(vm) > 1e-12) {
        let tc = clamp(-dot(q, m) / vm, t0, t1);
        let mid = depth_sq(f - d * tc, tan_half);
        if (mid < best) {
            best = mid;
            best_t = tc;
        }
    }
    if (best >= 1.0) {
        return vec4<f32>(0.0);
    }

    // Flux there, against the flux at the drawn length. `Ω` is `lc_world::signal::cone_solid_angle_sr`.
    let power = material.emission.x;
    let length_m = material.emission.z;
    let s = sin(0.5 * material.emission.y);
    let omega = 4.0 * PI * s * s;
    let at_end = power / (omega * length_m * length_m);
    // In lengths, so `log(flux / at_end)` is `-2 log(r)` with no watt in it: at 26 000 km a
    // meter squared is fourteen decades and the ratio of two of them loses nothing, but there
    // is no reason to ask.
    let r = max(length(f - d * best_t), 1e-6);
    let span = log(material.emission.w / at_end);
    var heat = 1.0;
    if (span > 0.0) {
        heat = clamp(-2.0 * log(r) / span, 0.0, 1.0);
    }

    // Deeper in the cone is more of it along the ray, so the edge feathers rather than being a
    // paper silhouette. The square root is a chord's profile across a disk.
    let across = sqrt(1.0 - best);
    let color = mix(material.faint.rgb, material.hot.rgb, heat) * across;

    // `AlphaMode::Add` is premultiplied, so alpha zero is what adds.
    return vec4<f32>(color, 0.0);
}
