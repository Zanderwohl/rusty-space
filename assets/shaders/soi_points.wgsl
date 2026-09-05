// Sphere-of-influence point cloud.
//
// The mesh is a unit Fibonacci sphere of billboard quads and is built once, ever: the
// radius arrives as a uniform and the vertex stage pushes each point out to the shell.
// A Hill sphere breathes over an eccentric orbit, so baking the radius into vertices
// would mean rebuilding the buffer every frame.
//
//   POSITION : unit direction from the body's centre (identical for all four corners)
//   CORNER   : quad corner in [-1, 1]^2
//
// Alpha is weighted toward the limb so the cloud reads as a shell rather than a fog:
// dense at the silhouette, nearly clear through the middle, so whatever is inside the
// sphere stays visible.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
}
#import exotic_matters::soi_shape::{SoiShape, soi_radius}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) direction: vec3<f32>,
    @location(1) corner: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) alpha: f32,
}

struct SoiPointsUniform {
    shape: SoiShape,
    base_color: vec4<f32>,
    point_angular_radius: f32,
    limb_power: f32,
    interior_alpha: f32,
    brightness: f32,
    emission_strength: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: SoiPointsUniform;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    let dir = normalize(vertex.direction);
    let radius = soi_radius(material.shape, dir);

    // The entity's transform is translation-only (the body's camera-relative position),
    // so this places the point at centre + radius * dir.
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let p_world = mesh_functions::mesh_position_local_to_world(
        world_from_local, vec4<f32>(dir * radius, 1.0)).xyz;

    // The camera sits at the render origin in this app's camera-relative scheme, so
    // view.world_position is ~zero and this is the true view vector.
    let view_vec = p_world - view.world_position.xyz;
    let distance = max(length(view_vec), 1e-9);

    // Constant angular size: a dot stays the same handful of pixels whether the shell is
    // Luna's or the Sun's.
    let size = distance * material.point_angular_radius;

    let pos_view = (view.view_from_world * vec4<f32>(p_world, 1.0)).xyz
        + vec3<f32>(vertex.corner, 0.0) * size;
    // Real depth, unlike the starfield's far-plane pin: a body in front must occlude
    // the points behind it.
    out.clip_position = view.clip_from_view * vec4<f32>(pos_view, 1.0);

    // The shell's outward normal is the direction itself. At the limb it is perpendicular
    // to the view, so this goes to zero exactly where the silhouette is.
    let normal = normalize(mesh_functions::mesh_normal_local_to_world(dir, vertex.instance_index));
    let ndv = abs(dot(normalize(view_vec), normal));
    let limb = pow(1.0 - ndv, material.limb_power);

    out.corner = vertex.corner;
    out.alpha = mix(material.interior_alpha, 1.0, limb) * material.shape.fade;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let r = length(in.corner);
    if (r > 1.0) {
        discard;   // a square quad, drawn as a round dot
    }
    let falloff = 1.0 - r * r;

    let emissive = material.base_color.rgb
        * material.brightness
        * material.emission_strength
        * in.alpha
        * falloff;
    return vec4<f32>(emissive, in.alpha * falloff);
}
