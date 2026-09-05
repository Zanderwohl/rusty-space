// Sphere-of-influence limb ring: the shell's silhouette, as a tube.
//
// The mesh stores angles, not positions — station angle `phi` around the ring in POSITION,
// cross-section angle `c` in NORMAL — so it is built once and never rebuilt, no matter how
// the radius breathes or the camera moves.
//
// For an isotropic shell the silhouette is a circle behind the centre, at the classic
// angular radius asin(R/D). For an anisotropic one it is a closed but NON-planar curve, so
// each station solves for its own tangent point.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}
#import exotic_matters::soi_shape::{SoiShape, soi_radius}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) station: vec3<f32>,   // (cos phi, sin phi, 0)
    @location(1) section: vec3<f32>,   // (cos c,   sin c,   0)
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) alpha: f32,
}

struct SoiRingUniform {
    shape: SoiShape,
    base_color: vec4<f32>,
    tube_radius: f32,
    emission_strength: f32,
    ring_alpha: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: SoiRingUniform;

/// Tangency residual for a station.
///
/// With the surface written as p(theta) = R(theta) * (e cos theta + w sin theta), the view
/// ray grazes it where the ray and the surface tangent are parallel. The cross-product
/// terms cancel, leaving:
///
///     f(theta) = R^2 - D * (R cos theta + R' sin theta)
///
/// which is zero at the silhouette. R' is a central difference, so the shape function only
/// ever has to answer "what is the radius toward this direction".
fn tangency(shape: SoiShape, e: vec3<f32>, w: vec3<f32>, distance: f32, theta: f32) -> f32 {
    let h = 1.0e-3;
    let r = soi_radius(shape, e * cos(theta) + w * sin(theta));
    let r_plus = soi_radius(shape, e * cos(theta + h) + w * sin(theta + h));
    let r_minus = soi_radius(shape, e * cos(theta - h) + w * sin(theta - h));
    let d_r = (r_plus - r_minus) / (2.0 * h);
    return r * r - distance * (r * cos(theta) + d_r * sin(theta));
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let centre = mesh_functions::mesh_position_local_to_world(
        world_from_local, vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;

    // The camera is at the render origin under this app's camera-relative scheme.
    let to_camera = view.world_position.xyz - centre;
    let distance = length(to_camera);

    // Inside the shell there is no silhouette at all. Collapse to a degenerate triangle
    // rather than draw a garbage ring; the CPU also hides the entity, and this covers the
    // frame where that decision is one step behind.
    if (distance <= material.shape.bounding_radius) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.alpha = 0.0;
        return out;
    }

    let e = to_camera / distance;

    // An orthonormal frame across the view direction. Matches `perpendicular_vectors`.
    var not_parallel = vec3<f32>(1.0, 0.0, 0.0);
    if (abs(e.x) >= 0.9) {
        not_parallel = vec3<f32>(0.0, 1.0, 0.0);
    }
    let u = normalize(cross(e, not_parallel));
    let v = cross(e, u);

    let cos_phi = vertex.station.x;
    let sin_phi = vertex.station.y;
    let w = u * cos_phi + v * sin_phi;

    // Closed form for a sphere: exact, and a seed good to a couple of degrees otherwise.
    let seed_radius = soi_radius(material.shape, w);
    var theta = acos(clamp(seed_radius / distance, -1.0, 1.0));

    if (material.shape.anisotropic > 0.5) {
        // Secant rather than Newton: it needs no second derivative, so the shape function's
        // contract stays "give me R(dir)". Two steps is far inside a pixel at ~13% squash.
        var t0 = theta;
        var t1 = theta + 1.0e-2;
        var f0 = tangency(material.shape, e, w, distance, t0);
        var f1 = tangency(material.shape, e, w, distance, t1);
        for (var i = 0; i < 2; i = i + 1) {
            let denom = f1 - f0;
            if (abs(denom) < 1.0e-12) {
                break;
            }
            let t2 = t1 - f1 * (t1 - t0) / denom;
            t0 = t1;
            f0 = f1;
            t1 = clamp(t2, 0.0, 3.14159265);
            f1 = tangency(material.shape, e, w, distance, t1);
        }
        theta = t1;
    }

    let dir = e * cos(theta) + w * sin(theta);
    let ring_point = centre + soi_radius(material.shape, dir) * dir;

    // The ring's own tangent, for the tube's cross-section frame. Exact for a sphere and
    // within a fraction of a degree for the squashed shell — invisible on a tube a few
    // pixels wide.
    let tangent = normalize((-u * sin_phi + v * cos_phi) * sin(theta));
    let n1 = normalize(cross(tangent, e));
    let n2 = cross(tangent, n1);

    // No base-radius subtraction as in trajectory.wgsl: this mesh bakes only angles, so
    // the tube radius comes straight from the uniform.
    let offset = (n1 * vertex.section.x + n2 * vertex.section.y) * material.tube_radius;

    out.clip_position = position_world_to_clip(ring_point + offset);
    out.alpha = material.ring_alpha * material.shape.fade;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let emissive = material.base_color.rgb * material.emission_strength * in.alpha;
    return vec4<f32>(emissive, in.alpha);
}
