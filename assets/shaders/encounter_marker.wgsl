// Encounter marker: three concentric circles at a sphere-of-influence crossing.
//
// Everything about the marker is angular, so it holds the same size on screen at any zoom:
// the circle radii and the tube thickness are both multiplied by the distance to the
// camera in the vertex stage. The mesh itself is pure angles and is built once.
//
//   POSITION : (cos phi, sin phi, ring index) — phi is the station around the circle
//   NORMAL   : (cos c,   sin c,   0)         — c is the angle around the tube
//
// The face of the target is spanned by `plane_x` and `plane_y`, both square to the
// traveller's velocity, so the craft flies into the target rather than along it.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) station: vec3<f32>,
    @location(1) section: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) alpha: f32,
}

struct EncounterMarkerUniform {
    // xyz used; w unused. Both unit, perpendicular, in render space.
    plane_x: vec4<f32>,
    plane_y: vec4<f32>,
    base_color: vec4<f32>,
    // Angular radius of each of the three circles, radians. w unused.
    ring_radii: vec4<f32>,
    tube_radius: f32,
    emission_strength: f32,
    alpha: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: EncounterMarkerUniform;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let centre = mesh_functions::mesh_position_local_to_world(
        world_from_local, vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;

    // The camera sits at the render origin in this app's camera-relative scheme.
    let distance = max(length(centre - view.world_position.xyz), 1e-9);

    // Angle times distance is a constant number of pixels, whatever the zoom.
    let ring = u32(vertex.station.z + 0.5);
    var ring_angle = material.ring_radii.x;
    if (ring == 1u) {
        ring_angle = material.ring_radii.y;
    } else if (ring == 2u) {
        ring_angle = material.ring_radii.z;
    }
    let radius = distance * ring_angle;
    let tube = distance * material.tube_radius;

    let cos_phi = vertex.station.x;
    let sin_phi = vertex.station.y;
    let radial = material.plane_x.xyz * cos_phi + material.plane_y.xyz * sin_phi;
    let point = centre + radial * radius;

    // Tube frame: around the circle, the two directions perpendicular to its tangent are
    // the outward radial and the plane's own normal.
    let normal = normalize(cross(material.plane_x.xyz, material.plane_y.xyz));
    let offset = (radial * vertex.section.x + normal * vertex.section.y) * tube;

    out.clip_position = position_world_to_clip(point + offset);
    out.alpha = material.alpha;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let emissive = material.base_color.rgb * material.emission_strength * in.alpha;
    return vec4<f32>(emissive, in.alpha);
}
