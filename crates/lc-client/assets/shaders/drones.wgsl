// Drones as stateless particles: each quad's position is a closed form of its index, the seed and
// the clock, so the picture at `t` is the same however often it is drawn. See
// em_render::drone_material and lightcone/docs/32-ship-rendering.md §Drones.
//
// Randomness is integer hashing only. `fract(sin(x) * big)` differs between GPUs, and two
// machines must agree on where every drone is.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

// Must match em_render::drone_material.
const MAX_DOCKS: u32 = 16u;
const MAX_TARGETS: u32 = 64u;

const TAU: f32 = 6.28318530718;
const PI: f32 = 3.14159265359;

// A mote's profile, `exp(-FALLOFF r^2)`, is cut at the quad's edge; this is its value there.
const FALLOFF: f32 = 4.0;
const EDGE: f32 = 0.01831563888;

// Fraction of a trip at each end over which a drone comes out of its dock or goes back in, so
// it does not appear from nowhere.
const HANGAR: f32 = 0.03;

struct DroneUniform {
    docks: array<vec4<f32>, MAX_DOCKS>,
    targets: array<vec4<f32>, MAX_TARGETS>,
    patrol_center: vec4<f32>,
    patrol_radii: vec4<f32>,
    color: vec4<f32>,
    carry_color: vec4<f32>,
    time: f32,
    seed: u32,
    count: u32,
    dock_count: u32,
    target_count: u32,
    working: f32,
    patrol: f32,
    carrying: f32,
    cycle_s: f32,
    dwell: f32,
    arc_lift: f32,
    hover_m: f32,
    patrol_period_s: f32,
    mote_m: f32,
    haze_m: f32,
    haze_px: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: DroneUniform;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    // Corner in `xy`, the drone's index in `z`.
    @location(0) position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) @interpolate(flat) rgb: vec3<f32>,
}

// PCG, from Jarzynski and Olano, "Hash Functions for GPU Rendering" (2020).
fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// `[0, 1)`, from the top 24 bits so it is exact in f32.
fn unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

fn unit_vector(h: u32) -> vec3<f32> {
    let z = 2.0 * unit(pcg(h)) - 1.0;
    let phi = TAU * unit(pcg(h ^ 0x68bc21ebu));
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3<f32>(r * cos(phi), r * sin(phi), z);
}

fn bezier(a: vec3<f32>, control: vec3<f32>, b: vec3<f32>, s: f32) -> vec3<f32> {
    let u = 1.0 - s;
    return u * u * a + 2.0 * u * s * control + s * s * b;
}

// The arc's control point: bowed away from the patrol center so a drone flies around the hull
// rather than through it, and pushed sideways by `h` so a hundred drones on one route fan out.
fn arc_control(a: vec3<f32>, b: vec3<f32>, h: u32) -> vec3<f32> {
    let mid = 0.5 * (a + b);
    let chord = length(b - a);
    let away = mid - material.patrol_center.xyz;
    let outward = select(vec3<f32>(0.0, 1.0, 0.0), normalize(away), dot(away, away) > 1e-12);
    let lift = material.arc_lift * chord;
    return mid + outward * lift + unit_vector(h) * (0.4 * lift);
}

struct Placed {
    position: vec3<f32>,
    // How much of the drone is out of its dock, `0..1`.
    presence: f32,
    // How brightly it glows in `carry_color`.
    glow: f32,
}

fn working(key: u32) -> Placed {
    var out: Placed;
    let period = material.cycle_s * (0.75 + 0.5 * unit(pcg(key + 1u)));
    let clock = material.time / period + unit(pcg(key + 2u));
    let trip_index = floor(clock);
    let p = clock - trip_index;
    // A new target every trip. The switch happens at the dock, where the path is continuous.
    let trip = pcg(key ^ pcg(bitcast<u32>(i32(trip_index))));

    let dock = material.docks[key % max(material.dock_count, 1u)].xyz;
    let goal = material.targets[trip % max(material.target_count, 1u)].xyz;

    let dwell = clamp(material.dwell, 0.0, 0.9);
    let leg = 0.5 * (1.0 - dwell);
    let carry = clamp(material.carrying, 0.0, 1.0);

    out.presence = smoothstep(0.0, HANGAR, p) * smoothstep(0.0, HANGAR, 1.0 - p);
    if (p < leg) {
        let s = smoothstep(0.0, 1.0, p / leg);
        out.position = bezier(dock, arc_control(dock, goal, trip + 11u), goal, s);
        out.glow = 0.0;
    } else if (p < leg + dwell) {
        let u = (p - leg) / dwell;
        // Zero with zero velocity at both ends, to match the arcs, which ease in and out.
        let wander = sin(PI * u) * sin(PI * u) * material.hover_m;
        let phases = vec3<f32>(unit(pcg(trip + 21u)), unit(pcg(trip + 22u)), unit(pcg(trip + 23u)));
        out.position = goal + wander * sin(TAU * (vec3<f32>(1.0, 1.7, 2.3) * u + phases));
        // The piece is picked up at the end of the dwell.
        out.glow = carry * smoothstep(0.6, 1.0, u);
    } else {
        let s = smoothstep(0.0, 1.0, (p - leg - dwell) / leg);
        out.position = bezier(goal, arc_control(goal, dock, trip + 31u), dock, s);
        out.glow = carry * (1.0 - smoothstep(0.85, 1.0, s));
    }
    return out;
}

// A circle over the patrol ellipsoid, just outside it, breathing slowly in and out.
fn patrolling(key: u32) -> Placed {
    var out: Placed;
    let start = unit_vector(key + 41u);
    let axis = normalize(cross(start, unit_vector(key + 42u)) + vec3<f32>(1e-6, 0.0, 0.0));
    let period = material.patrol_period_s * (0.7 + 0.6 * unit(pcg(key + 43u)));
    let angle = TAU * (material.time / period);
    let dir = start * cos(angle) + cross(axis, start) * sin(angle);
    let breathe = 1.1 + 0.05 * sin(TAU * (material.time / (3.1 * period) + unit(pcg(key + 44u))));
    out.position = material.patrol_center.xyz + material.patrol_radii.xyz * dir * breathe;
    out.presence = 1.0;
    out.glow = 0.0;
    return out;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.corner = vertex.position.xy;
    out.rgb = vec3<f32>(0.0);

    let index = u32(vertex.position.z);
    let key = pcg(index ^ pcg(material.seed));
    let role = unit(pcg(key ^ 0x9e3779b9u));
    let busy = clamp(material.working, 0.0, 1.0);
    let patrol_above = 1.0 - clamp(material.patrol, 0.0, 1.0) * (1.0 - busy);

    var placed: Placed;
    if (index >= material.count) {
        placed.presence = 0.0;
    } else if (role < busy && material.dock_count > 0u && material.target_count > 0u) {
        placed = working(key);
    } else if (role >= patrol_above) {
        placed = patrolling(key);
    } else {
        placed.presence = 0.0;
    }
    if (placed.presence <= 0.0) {
        // Docked. Four identical corners are a quad of no area, so nothing is drawn.
        out.clip_position = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        return out;
    }

    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let center = (world_from_local * vec4<f32>(placed.position, 1.0)).xyz;
    let scale = length(world_from_local[0].xyz);

    // clip_from_view[1][1] is 1 / tan(fov_y / 2) for a perspective lens.
    let distance = max(length(center - view.world_position.xyz), 1e-6);
    let px_per_m = view.viewport.w * view.clip_from_view[1][1] / (2.0 * distance);
    let mote = material.mote_m * scale;
    let mote_px = mote * px_per_m;

    // Haze before a mote falls below a pixel, which would land on no pixel center and sparkle: the
    // same light spread over a disc wide enough to overlap its neighbors, and never narrower than
    // `haze_px`. The light is conserved, so the haze has the swarm's true brightness per pixel.
    let hazed = 1.0 - smoothstep(material.haze_px / 3.0, material.haze_px, mote_px);
    let haze = max(max(material.haze_m * scale, material.haze_px / px_per_m), mote);
    let size = mix(mote, haze, hazed);
    let conserve = (mote / size) * (mote / size);

    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let world = center + (right * vertex.position.x + up * vertex.position.y) * (0.5 * size);
    out.clip_position = position_world_to_clip(world);
    out.rgb = mix(material.color.rgb, material.carry_color.rgb, placed.glow) * placed.presence * conserve;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let r2 = dot(in.corner, in.corner);
    if (r2 >= 1.0) {
        discard;
    }
    let profile = (exp(-FALLOFF * r2) - EDGE) / (1.0 - EDGE);
    return vec4<f32>(in.rgb * profile, 0.0);
}
