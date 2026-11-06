// Obsidian: knapped volcanic glass, blue-gray-black, all reflection, cut by scars.wgsl's flake
// scars on the mesh's own position, so any shape takes them without a seam and they stay on the
// part as it moves. Look development: crates/lc-client/src/looks/obsidian.rs; hull.wgsl draws the
// same glass in the ship's frame.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::{globals, view},
    view_transformations::position_world_to_clip,
}
#import "shaders/hash.wgsl"::{pcg, unit}
#import "shaders/scars.wgsl"::{glass_specular, knapped}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    // In the mesh's own frame, where the scars are cut, and that frame's axes in the world's.
    @location(2) local_position: vec3<f32>,
    @location(3) local_normal: vec3<f32>,
    @location(4) @interpolate(flat) axis_x: vec3<f32>,
    @location(5) @interpolate(flat) axis_y: vec3<f32>,
    @location(6) @interpolate(flat) axis_z: vec3<f32>,
}

struct ObsidianUniform {
    /// `(scar_m, dish, tilt_rad, ripple)`: the larger scars' spacing, how far a dish's normal leans
    /// toward its middle at its rim, each scar's own tilt, and the ripples' lean.
    scar: vec4<f32>,
    /// The glass's color, linear, and its roughness in `w`.
    glass: vec4<f32>,
    /// Toward the star, and its turn in `w`.
    to_star: vec4<f32>,
    star_color: vec4<f32>,
    /// Direction to the planet's center, and its angular radius in `w`, radians.
    to_planet: vec4<f32>,
    planet_color: vec4<f32>,
    /// What the glass mirrors of the sky: dim above, dimmer below.
    sky: vec4<f32>,
    /// `(exposure, seed, 0, 0)`.
    finish: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: ObsidianUniform;

const PI: f32 = 3.14159265;
/// Obsidian's index is about 1.49.
const F0: f32 = 0.04;

/// The star, turned about world Y at `to_star.w` rad/s.
fn turned(to_star: vec4<f32>) -> vec3<f32> {
    let a = to_star.w * globals.time;
    let d = normalize(to_star.xyz);
    return vec3<f32>(cos(a) * d.x - sin(a) * d.z, d.y, sin(a) * d.x + cos(a) * d.z);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_position = world.xyz;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    out.local_position = vertex.position;
    out.local_normal = vertex.normal;
    out.axis_x = normalize(mesh_functions::mesh_normal_local_to_world(vec3<f32>(1.0, 0.0, 0.0), vertex.instance_index));
    out.axis_y = normalize(mesh_functions::mesh_normal_local_to_world(vec3<f32>(0.0, 1.0, 0.0), vertex.instance_index));
    out.axis_z = normalize(mesh_functions::mesh_normal_local_to_world(vec3<f32>(0.0, 0.0, 1.0), vertex.instance_index));
    return out;
}

/// The planet seen along `r`, its limb blurred by the roughness.
fn planet_seen(r: vec3<f32>, roughness: f32) -> f32 {
    let off = acos(clamp(dot(r, normalize(m.to_planet.xyz)), -1.0, 1.0));
    let blur = 0.5 * roughness * roughness + 0.01;
    return 1.0 - smoothstep(m.to_planet.w - blur, m.to_planet.w + blur, off);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let surface = normalize(in.world_normal);
    let e = normalize(view.world_position - in.world_position);
    let size = m.scar.x;
    let p = in.local_position;
    let pixel = max(length(fwidth(p)), 1.0e-4);
    let k = knapped(p, normalize(in.local_normal), size, m.scar.yzw, pixel, u32(m.finish.y));
    let n = normalize(in.axis_x * k.normal.x + in.axis_y * k.normal.y + in.axis_z * k.normal.z);
    let roughness = mix(0.5, m.glass.w, k.keep_fine);

    // A ridge is sharp: the edge itself catches a thin line of light whichever way it faces.
    let edge = 1.0 - smoothstep(0.0, max(0.03 * size, pixel), k.coarse.ridge);
    let tint = m.glass.rgb * mix(0.8, 1.2, unit(pcg(k.coarse.id + 3u)));

    let ne = max(dot(n, e), 0.0);
    let fresnel = F0 + (1.0 - F0) * pow(1.0 - ne, 5.0);
    let r = reflect(-e, n);
    var radiance = m.sky.rgb * fresnel * mix(0.3, 1.0, 0.5 + 0.5 * r.y);
    radiance += m.planet_color.rgb * fresnel * planet_seen(r, roughness);
    let star = turned(m.to_star);
    if (dot(star, surface) > 0.0) {
        let ndl = max(dot(n, star), 0.0);
        radiance += m.star_color.rgb * (tint * ndl / PI + glass_specular(n, star, e, roughness, F0));
        radiance += m.star_color.rgb * 0.003 * edge * k.keep_coarse * max(dot(surface, star), 0.0);
    }
    return vec4<f32>(radiance * m.finish.x, 1.0);
}
