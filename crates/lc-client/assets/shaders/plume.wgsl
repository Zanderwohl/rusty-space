// A drive's exhaust: a volume of glowing gas, integrated along the view ray.
//
// The mesh is a proxy and its shape never shows. What is drawn is a cone of gas inside it,
// widening from the nozzle and thinning as it goes, and every fragment works out how much of
// that cone its own ray passes through. That is where the feathered edge comes from: a ray
// grazing the side crosses almost nothing, one down the middle crosses the lot.
//
// Local space is the proxy's. The axis is `+y`, running from `-0.5` at the nozzle to `+0.5` at
// the far end, and a radius of one is the proxy wall.
//
// The gas is not uniform. A drive burns fuel-rich, and what leaves the injector unmixed is
// drawn out by the flow into lengthwise streaks of cooler, sootier gas — so every sample is
// split into two gases with two colors rather than scaled by one, and the streaks travel down
// the plume with the clock.

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

struct PlumeUniform {
    glow: vec4<f32>,
    /// `(throat, mouth, edge, taper)`.
    shape: vec4<f32>,
    eye_local: vec4<f32>,
    /// `(surface_reference, stops, brightness, overflow)`.
    exposure: vec4<f32>,
    soot: vec4<f32>,
    /// `(phase, across, along, bite)`. The phase is in lattice cells of the first octave.
    churn: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: PlumeUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var churn: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var churn_sampler: sampler;

const STEPS: i32 = 24;
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

/// The churn's period, in lattice cells of its first octave, on every axis.
///
/// The baked churn repeats on it, so the host can wrap the phase there and the pattern does not
/// jump. Mirrored by `em_render::plume_material::CHURN_PERIOD`, and by `textures/plume.tgraph`.
const CHURN_PERIOD: f32 = 32.0;

/// Which part of the noise counts as a fuel-rich lane.
///
/// A window rather than the noise itself: used raw, every parcel is a bit rich and the plume is
/// evenly dirty. Taking the top of the distribution gives lanes with clean gas between them,
/// which is what a striated plume actually looks like.
const RICH_LOW: f32 = 0.35;
const RICH_HIGH: f32 = 0.80;

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

/// How fuel-rich the gas is at a point: zero is burnt clean, one is a streak of soot.
///
/// Sampled on `(where the point sits across the cone, how far along it is)` — the cross-section
/// **in units of the local radius** rather than in meters. That coordinate is constant along a
/// streamline, because a parcel that leaves the injector a third of the way out stays a third of
/// the way out while the cone flares around it. So the pattern is a bundle of filaments running
/// the length of the plume, widening with it, and traveling aft as the phase slides. Sampling
/// the raw position instead gives blobs of dirt hanging still in the proxy while the ship
/// maneuveres round them.
///
/// **Filaments and not sheets.** The first go used only the *direction* across the cone, which
/// makes each lane a full radial sheet — and a ray down the middle of the plume crosses every
/// angle there is, averages the lot, and comes out the color of clean gas. Only the grazing
/// rays at the silhouette kept any contrast, so the plume had a fringe and a blank middle.
/// Localizing a lane in the cross-section means every ray crosses a few of them and none of it
/// averages flat.
///
/// The noise itself is `textures/plume.tgraph`, two octaves baked into a volume that repeats on
/// every axis. Level zero, explicitly: the march samples inside a loop, where an implicit
/// derivative is not allowed.
fn richness(p: vec3<f32>, radius: f32, along: f32) -> f32 {
    let flat = p.xz / max(radius, 1e-6);
    let coord = vec3<f32>(flat * material.churn.y, along * material.churn.z - material.churn.x);
    let n = textureSampleLevel(churn, churn_sampler, coord / CHURN_PERIOD, 0.0).r;
    return smoothstep(RICH_LOW, RICH_HIGH, n);
}

/// How much gas is at a point of the proxy, split into `(clean, rich)`, in arbitrary units.
///
/// Zero outside the length, and a smooth falloff to the side rather than a wall — a plume in
/// vacuum has no boundary, it just runs out of gas.
///
/// The split conserves the column: the streaks move gas from one color to the other and never
/// destroy it, so the core stays where the exposure was set for it and the plume does not dim
/// when the churn is turned up.
fn gas(p: vec3<f32>) -> vec2<f32> {
    let along = p.y + 0.5;
    if (along < 0.0 || along > 1.0) {
        return vec2<f32>(0.0);
    }
    // The cone: from the nozzle out to the mouth. This is the expansion ratio, drawn.
    let radius = mix(material.shape.x, material.shape.y, along);
    let across = length(p.xz) / max(radius, 1e-6);
    // Gaussian across, so the edge feathers instead of cutting.
    let profile = exp(-across * across * material.shape.z);
    // Thinning along, as it spreads and cools.
    //
    // No `1/r^2` on top of it: that was a second go at the same idea, and between them the
    // column came out a hundred times deeper at the nozzle than at the mouth, which put every
    // part of the plume above the top of the exposure window and turned the whole cone into one
    // flat saturated shape. Bounded by one here, so the depth a ray accumulates is of order the
    // distance it traveled and the brightness scale below means something.
    let fading = pow(max(1.0 - along, 0.0), material.shape.w);
    let amount = profile * fading;
    // Most samples of a convex volume are outside it, and skipping them skips the fetch.
    if (amount < 1e-4) {
        return vec2<f32>(amount, 0.0);
    }

    // Nothing at the throat: the flow has not run far enough there to have separated into
    // anything, and a plume that is striated the moment it leaves the nozzle looks like a
    // painted cone rather than like gas coming apart.
    let bite = material.churn.w * smoothstep(0.0, 0.2, along);
    let rich = richness(p, radius, along) * bite;
    return vec2<f32>(amount * (1.0 - rich), amount * rich);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // The ray, in the proxy's own space. Back faces are what is drawn, so this fragment is on
    // the wall *behind* the gas and the march runs from here back toward the eye.
    //
    // **Backwards, and that is not a preference.** A plume is meters long and sits an
    // astronomical unit from the render origin, so the eye is of order `1e8` in the proxy's own
    // units — and `eye + direction * t` then asks `f32` for a point near the origin as the
    // difference of two numbers near `1e8`, where its spacing is about eight. Every position
    // the density is sampled at comes out quantized to nothing and the plume does not appear at
    // all. Starting from the fragment keeps every term of order one.
    let eye = material.eye_local.xyz;
    let toward = in.local - eye;
    let range = length(toward);
    if (range <= 0.0) {
        return vec4<f32>(0.0);
    }
    let direction = toward / range;

    // How far back the ray stays inside the slab the gas lives in. A ray along the axis never
    // leaves it, and is bounded by the proxy's own width instead.
    var span = 2.0;
    if (abs(direction.y) > 1e-6) {
        let leaves = select(in.local.y - 0.5, in.local.y + 0.5, direction.y > 0.0);
        span = abs(leaves / direction.y);
    }
    // And never further than the eye itself, for when the camera is inside the proxy.
    span = min(span, range);
    if (span <= 0.0) {
        return vec4<f32>(0.0);
    }

    // Emission only, with no absorption: the gas is thin and it is the brightest thing in the
    // frame, so what reaches the eye is the sum of what every part of it puts out.
    //
    // Two columns rather than one, because the ray crosses two gases and they are not the same
    // color. Summing the depth first and coloring it afterwards would average the streaks
    // away — a ray through the flank crosses several lanes and the mean of a lane and the gas
    // beside it is the gas beside it.
    let step = span / f32(STEPS);
    var depth = vec2<f32>(0.0);
    for (var i = 0; i < STEPS; i = i + 1) {
        let back = (f32(i) + 0.5) * step;
        depth = depth + gas(in.local - direction * back) * step;
    }
    if (depth.x + depth.y <= 0.0) {
        return vec4<f32>(0.0);
    }

    let linear = (material.glow.rgb * depth.x + material.soot.rgb * depth.y)
        * material.exposure.z;

    // The tone map of crate::tonemap, the same curve the lit surfaces evaluate, so a plume and
    // the planet it is flying past sit in one exposure rather than two that agree.
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

    // Overflow leaves as an HDR value rather than clipping, exactly as a star's does in
    // `starfield.wgsl`. It is what makes the column *read* as a column: the depth through a
    // plume varies by decades between a ray down the axis and one grazing the flank, and
    // clipped at one the whole of that range came back as the same pale lavender — hue and
    // saturation are held constant by the curve, so a brighter part of the plume was only a
    // brighter lavender and the deep middle looked like the thin edge. Carried past one, the
    // display transform desaturates the core toward white and bloom haloes it, while the
    // edges stay inside the window with their color intact.
    //
    // **Inside the window the curve keeps the color; past it, each channel is on its own.**
    // That is the one place a sensor and this tone map part company, and the plume is where it
    // matters: a channel does not know what the other two are doing, so it saturates when *it*
    // is full. Under a false-color mapping the three are decades apart — in `thermal` this
    // plume's channels span nearly four stops around their own luminance, because ten microns,
    // two microns and green are three quite different questions to ask a fifty-thousand-kelvin
    // gas. Scaling one chroma by one overflow asks only the brightest of them and reports the
    // answer as the color of all three, which is how the hottest object in the frame came back
    // a flat saturated blue. Per channel, the blue fills first, then green, then red, and the
    // core goes white the way something too bright to photograph does.
    //
    // The epsilon is to keep `log2` off zero; the cap is so a degenerate exposure gives a
    // bright plume rather than an infinite one.
    let over = clamp(
        log2(max(linear, vec3<f32>(1e-30)) / max(reference, 1e-30)),
        vec3<f32>(0.0),
        vec3<f32>(12.0),
    );

    // **Alpha zero.** `AlphaMode::Add` is premultiplied — `src + dst * (1 - alpha)` — so
    // anything else here would rub out what is behind the plume instead of adding to it.
    return vec4<f32>(chroma * level + over * material.exposure.w, 0.0);
}
