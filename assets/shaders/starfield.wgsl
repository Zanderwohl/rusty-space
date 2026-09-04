// Starfield background shader.
//
// Each star is drawn as a single camera-facing billboard quad (2 triangles)
// instead of scanning every catalog star for every fragment. The quad center
// sits on the unit sky sphere along the star's precomputed direction (the
// camera is fixed at the world origin in this app's camera-relative scheme),
// and the corners are expanded in view space by the star's angular radius.
//
// Per-vertex data baked on the CPU:
//   POSITION : unit direction to the star (Bevy Y-up)
//   COLOR    : vec4(linear_rgb, baked_brightness)   -- brightness folded into .a
//   CORNER   : quad corner in [-1, 1]^2 (also the falloff coordinate)
//   SIZE_T   : 0..1 size factor from magnitude (bright stars -> 1 -> max radius)

#import bevy_pbr::mesh_view_bindings::view

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) corner: vec2<f32>,
    @location(3) size_t: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) color: vec4<f32>,
}

struct StarfieldMaterialUniform {
    star_count: u32,
    brightness: f32,
    star_radius_min: f32,
    star_radius_max: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: StarfieldMaterialUniform;

// Arcminutes -> radians (pi / (180 * 60)).
const ARCMIN_TO_RAD: f32 = 0.0002908882;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    // Star direction in view space. Using w=0 makes this translation-invariant,
    // so the starfield depends only on camera orientation. `vertex.position` is
    // already a unit direction (baked from the catalog), so no input normalize is
    // needed; one output normalize guards against any rounding drift.
    let dir_view = normalize((view.view_from_world * vec4<f32>(vertex.position, 0.0)).xyz);

    // Per-star angular radius (brighter stars are drawn larger).
    let radius_rad = mix(material.star_radius_min, material.star_radius_max, vertex.size_t) * ARCMIN_TO_RAD;

    // Billboard: center sits one unit down the view direction; corners are
    // offset in the view XY plane. At unit distance the lateral offset equals
    // the subtended angle, so the quad spans the star's angular diameter.
    let pos_view = dir_view + vec3<f32>(vertex.corner, 0.0) * radius_rad;

    var clip = view.clip_from_view * vec4<f32>(pos_view, 1.0);

    // Clamp to background depth so the starfield renders behind the scene.
    clip.z = clip.w;

    out.clip_position = clip;
    out.corner = vertex.corner;
    out.color = vertex.color;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Radial falloff turns the square quad into a soft disc.
    let r = length(in.corner);
    if (r > 1.0) {
        discard;
    }
    let falloff = 1.0 - r;

    let brightness = in.color.a;
    let emissive = in.color.rgb * brightness * falloff * material.brightness;
    return vec4<f32>(emissive, 1.0);
}
