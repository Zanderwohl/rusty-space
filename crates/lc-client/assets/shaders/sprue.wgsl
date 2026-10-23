// A layered sprue: an opaque plated base and two cutout layers of pipe over it, each flat and
// normal-mapped, standing off one another. Lit by lamps in the gaps between layers. Look
// development only: crates/lc-client/examples/sprue_void.
//
// Each face is mapped in its own frame (u, v, n), meters, from world position, so the three
// layers of one face share coordinates and a lamp's place is the same on all of them. The
// mesh must be axis-aligned boxes.

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
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
}

struct SprueUniform {
    /// `(layer, tile_m, bulb_radius_m, bulb_power)`.
    layer: vec4<f32>,
    /// Meters over the base: layer 1, layer 2, the deep lamps, the lamps riding layer 1.
    heights: vec4<f32>,
    /// `(cell_m, deep share, riding share, scatter)`.
    lamps: vec4<f32>,
    /// Linear color times power at a meter.
    lamp_color: vec4<f32>,
    to_star: vec4<f32>,
    star_color: vec4<f32>,
    ambient: vec4<f32>,
    /// `(exposure, gloss, specular, 0)`.
    finish: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: SprueUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var tiles: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var tile_sampler: sampler;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_position = world.xyz;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    return out;
}

struct Face {
    u: vec3<f32>,
    v: vec3<f32>,
    n: vec3<f32>,
    id: f32,
}

fn face(normal: vec3<f32>) -> Face {
    let n = normalize(normal);
    let up = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.z) > 0.9);
    let u = normalize(cross(up, n));
    return Face(u, cross(n, u), n, dot(round(n), vec3<f32>(1.0, 3.0, 9.0)));
}

fn to_face(f: Face, w: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(w, f.u), dot(w, f.v), dot(w, f.n));
}

fn hash4(p: vec3<f32>) -> vec4<f32> {
    var p4 = fract(p.xyzx * vec4<f32>(0.1031, 0.1030, 0.0973, 0.1099));
    p4 = p4 + dot(p4, p4.wzxy + 33.33);
    return fract((p4.xxyz + p4.yzzw) * p4.zywx);
}

fn layer_height(k: u32) -> f32 {
    if (k == 1u) { return m.heights.x; }
    if (k == 2u) { return m.heights.y; }
    return 0.0;
}

/// Coverage of layer `k` at `p` meters, softened by the mip `level`.
fn cover(p: vec2<f32>, k: u32, level: f32) -> f32 {
    return textureSampleLevel(tiles, tile_sampler, p / m.layer.y, k, level).a;
}

/// What reaches a point at height `z` from height `zl` along `p → q`, past the cutout layers
/// between them.
fn passed(p: vec2<f32>, z: f32, q: vec2<f32>, zl: f32, level: f32) -> f32 {
    var through = 1.0;
    for (var k = 1u; k <= 2u; k++) {
        let zk = layer_height(k);
        if ((zk - z) * (zk - zl) < 0.0) {
            let at = mix(p, q, (zk - z) / (zl - z));
            through *= 1.0 - smoothstep(0.35, 0.65, cover(at, k, level));
        }
    }
    return through;
}

struct Lamp {
    on: bool,
    at: vec2<f32>,
    tint: vec3<f32>,
}

/// Cell `cell`'s lamp in set `which`; a riding lamp exists only where layer 1 is there to carry it.
fn lamp(f: Face, cell: vec2<f32>, which: f32) -> Lamp {
    let h = hash4(vec3<f32>(cell, f.id * 17.0 + which * 101.0));
    let share = select(m.lamps.y, m.lamps.z, which > 0.5);
    let at = (cell + 0.2 + 0.6 * h.yz) * m.lamps.x;
    var on = h.x < share;
    if (on && which > 0.5) {
        on = cover(at, 1u, 0.0) > 0.5;
    }
    // Most are high-pressure sodium; a few run oranger or paler, as a yard's lamps do with age.
    let tint = mix(vec3<f32>(1.0, 0.85, 0.6), vec3<f32>(1.0, 1.08, 1.3), h.w * h.w);
    return Lamp(on, at, tint);
}

struct Lit {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
    glow: vec3<f32>,
}

fn lamps(f: Face, p: vec2<f32>, z: f32, n: vec3<f32>, to_eye: vec3<f32>, which: f32, zl: f32) -> Lit {
    var out = Lit(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0));
    let cell_m = m.lamps.x;
    let home = floor(p / cell_m);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let l = lamp(f, home + vec2<f32>(f32(dx), f32(dy)), which);
            if (!l.on) { continue; }
            let d = vec3<f32>(l.at - p, zl - z);
            let d2 = dot(d, d);
            // Out to a cell, so the nine cells about a point hold every lamp that reaches it.
            let window = pow(clamp(1.0 - d2 / (cell_m * cell_m), 0.0, 1.0), 2.0);
            if (window <= 0.0) { continue; }
            let to_lamp = normalize(f.u * d.x + f.v * d.y + f.n * d.z);
            let through = passed(p, z, l.at, zl, 1.5);
            let power = m.lamp_color.rgb * l.tint * window * through / (d2 + 0.04);
            let ndl = max(dot(n, to_lamp), 0.0);
            let half_way = normalize(to_lamp + to_eye);
            out.diffuse += power * ndl;
            out.specular += power * ndl * pow(max(dot(n, half_way), 0.0), m.finish.y);
            out.glow += power;
        }
    }
    return out;
}

/// The lamps themselves, seen from height `z` and drawn where the eye's ray crosses their
/// height, so they sit at their own depth among the layers.
fn bulbs(f: Face, p: vec2<f32>, z: f32, eye: vec3<f32>, which: f32, zl: f32) -> vec3<f32> {
    if (eye.z <= 1.0e-3) { return vec3<f32>(0.0); }
    let seen = p + eye.xy * (zl - z) / eye.z;
    let home = floor(seen / m.lamps.x);
    var out = vec3<f32>(0.0);
    let r = m.layer.z;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let l = lamp(f, home + vec2<f32>(f32(dx), f32(dy)), which);
            if (!l.on) { continue; }
            let d2 = dot(seen - l.at, seen - l.at) / (r * r);
            out += m.lamp_color.rgb * l.tint * m.layer.w * (exp(-d2 * 4.0) + 0.04 * exp(-d2 * 0.15));
        }
    }
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let f = face(in.world_normal);
    let k = u32(m.layer.x + 0.5);
    let p = vec2<f32>(dot(in.world_position, f.u), dot(in.world_position, f.v));
    let texel = textureSample(tiles, tile_sampler, p / m.layer.y, k);
    if (k > 0u && texel.a < 0.5) {
        discard;
    }
    let tn = vec2<f32>(texel.r, texel.g) * 2.0 - 1.0;
    let n = normalize(f.u * tn.x + f.v * tn.y + f.n * sqrt(max(1.0 - dot(tn, tn), 0.0)));
    let albedo = texel.b;
    let z = layer_height(k);
    let to_eye = normalize(view.world_position - in.world_position);
    let eye = to_face(f, to_eye);

    var radiance = m.ambient.rgb * albedo;

    let star = normalize(m.to_star.xyz);
    let sf = to_face(f, star);
    if (sf.z > 0.0) {
        // The star is far: its ray climbs out of the sandwich along `sf`.
        let out_at = p + sf.xy * (m.heights.y + 1.0 - z) / sf.z;
        let through = passed(p, z, out_at, m.heights.y + 1.0, 1.0);
        let ndl = max(dot(n, star), 0.0);
        let half_way = normalize(star + to_eye);
        let spec = m.finish.z * pow(max(dot(n, half_way), 0.0), m.finish.y);
        radiance += m.star_color.rgb * through * ndl * (albedo + spec);
    }

    let deep = lamps(f, p, z, n, to_eye, 0.0, m.heights.z);
    let riding = lamps(f, p, z, n, to_eye, 1.0, m.heights.w);
    radiance += (deep.diffuse + riding.diffuse) * albedo;
    radiance += (deep.specular + riding.specular) * m.finish.z;
    radiance += (deep.glow + riding.glow) * m.lamps.w;

    if (k == 0u) {
        radiance += bulbs(f, p, z, eye, 0.0, m.heights.z);
    } else if (k == 1u) {
        radiance += bulbs(f, p, z, eye, 1.0, m.heights.w);
    }

    return vec4<f32>(radiance * m.finish.x, 1.0);
}
