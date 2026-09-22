// The map's lines, held at a constant width in pixels. `map_line::tube_radius` is the same
// rule in Rust, and is what the tests check.

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
    @location(6) center_before: vec3<f32>,
    @location(7) center_after: vec3<f32>,
    @location(8) arc: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(5) color: vec4<f32>,
    @location(6) arc: f32,
}

struct MapLineMaterial {
    base_color: vec4<f32>,
    emission_strength: f32,
    base_tube_radius: f32,
    max_fraction: f32,
    width_px: f32,
    dash_px: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: MapLineMaterial;

/// How far `point` is from the segment `a`-`b`. `map_line::nearest`.
fn nearest(point: vec3<f32>, a: vec3<f32>, b: vec3<f32>) -> f32 {
    let span = b - a;
    let length_squared = dot(span, span);
    var along = 0.0;
    if (length_squared > 1.17549435e-38) {
        along = clamp(dot(point - a, span) / length_squared, 0.0, 1.0);
    }
    return length(point - (a + span * along));
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);

    // Where this vertex's ring of the tube is centered. A disc has one normal and no tube, and
    // its cap is the base radius, so it is displaced by exactly nothing.
    let center = vertex.position - vertex.normal * material.base_tube_radius;
    let center_world = (world_from_local * vec4(center, 1.0)).xyz;
    let before_world = (world_from_local * vec4(vertex.center_before, 1.0)).xyz;
    let after_world = (world_from_local * vec4(vertex.center_after, 1.0)).xyz;
    // The nearer segment and not this vertex: a spoke is one segment, and the part of it passing
    // under the camera is nowhere near either end.
    let eye = view.world_position;
    let reach = min(nearest(eye, before_world, center_world),
        nearest(eye, center_world, after_world));

    // Radians a pixel subtends, from the projection: `clip_from_view[1][1]` is 1 / tan(fov / 2).
    let rad_per_px = 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    // Across the tube, which is x for every family: a drop line is stretched along y only.
    let scale = length(world_from_local[0].xyz);
    var radius = material.base_tube_radius;
    if (scale > 1.17549435e-38) {
        radius = min(max(reach * rad_per_px * material.width_px, 1.17549435e-38) / scale,
            material.max_fraction);
    }

    let world = world_from_local * vec4(center + vertex.normal * radius, 1.0);
    out.clip_position = position_world_to_clip(world.xyz);
    out.color = vertex.color;
    out.arc = vertex.arc;
    return out;
}

/// Lit for the first half of each period of `arc`, dark for the second.
fn lit_in(arc: f32, period: f32) -> f32 {
    return select(0.0, 1.0, fract(arc / period) < 0.5);
}

/// How much of a dash this fragment is in, `0..1`, for dashes about `dash_px` long on screen.
///
/// The pattern is laid along the mesh's own arc length, so it stays put on the line as the
/// camera moves. How much arc a pixel covers comes from the derivative, and it varies along a
/// line seen in perspective, so the pattern is two power-of-two periods either side of the one
/// wanted, faded by which is nearer: as one gives way every other gap fills in, so nothing
/// pops. The shorter lights dashes between half and all of `dash_px`, the longer between one
/// and two, and a bigger shape on screen has more dashes rather than longer ones.
fn dash(arc: f32) -> f32 {
    if (material.dash_px <= 0.0) {
        return 1.0;
    }
    let per_px = length(vec2<f32>(dpdx(arc), dpdy(arc)));
    if (per_px <= 0.0) {
        return 1.0;
    }
    // `map_line::dash_period`.
    let level = log2(2.0 * material.dash_px * per_px);
    let period = exp2(floor(level));
    return mix(lit_in(arc, period), lit_in(arc, 2.0 * period), fract(level));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Before any branch: derivatives want every fragment of the quad still running.
    let lit = dash(in.arc);
    if (lit <= 0.0) {
        discard;
    }
    let color = material.base_color.rgb * (1.0 + in.color.a * material.emission_strength);
    return vec4(color * lit, 1.0);
}
