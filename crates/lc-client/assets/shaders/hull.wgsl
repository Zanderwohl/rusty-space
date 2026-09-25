// A hull in regions: em_render::hull_material. lightcone/docs/32-ship-rendering.md is the look.
//
// The mesh's own position is the ship's frame in meters, so a tile is the same size in meters
// on any hull. Tiles are mipmapped and sampled with derivatives: detail below a pixel is drawn
// as its average rather than shimmering.
//
// Under construction (reveal.w below ALL_PLATED) each point reads its own bands from its distance
// to the joint. With HULL_GIRDER the mesh is the truss: tubes carrying a girder each, not regions.

#import bevy_pbr::{
    mesh_functions,
    view_transformations::position_world_to_clip,
}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
#ifdef HULL_GIRDER
    // em_render::hull_material::ATTRIBUTE_HULL_GIRDER: (x_m, threshold, scaffold, 0).
    @location(2) girder: vec4<f32>,
#else
    // em_render::hull_material::ATTRIBUTE_HULL_REGIONS: a weight a region.
    @location(2) regions_0: vec4<f32>,
    @location(3) regions_1: vec4<f32>,
    @location(4) regions_2: vec4<f32>,
    @location(5) regions_3: vec4<f32>,
    // em_render::hull_material::ATTRIBUTE_HULL_SEAM: signed meters across to the seam, and along it.
    @location(6) seam: vec2<f32>,
#endif
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) ship_position: vec3<f32>,
    @location(2) ship_normal: vec3<f32>,
#ifdef HULL_GIRDER
    @location(3) @interpolate(flat) girder: vec4<f32>,
#else
    @location(3) regions_0: vec4<f32>,
    @location(4) regions_1: vec4<f32>,
    @location(5) regions_2: vec4<f32>,
    @location(6) regions_3: vec4<f32>,
    @location(7) seam: vec2<f32>,
#endif
}

const REGIONS: u32 = 16u;

struct HullUniform {
    /// World direction to the star; `w` is the light on the unlit side.
    to_star: vec4<f32>,
    reflected: vec4<f32>,
    /// `(surface_reference, stops, 0, 0)`.
    exposure: vec4<f32>,
    /// `(tile_m, 0, 0, 0)`.
    detail: vec4<f32>,
    /// `(origin, front)`, ship frame, meters.
    reveal: vec4<f32>,
    /// `(panel_m, spread_m, 0, 0)`.
    reveal_panel: vec4<f32>,
    /// `(truss, fitting-out, scaffold down, span)`, meters from reveal's origin.
    build_fronts: vec4<f32>,
    build_widths: vec4<f32>,
    /// `(pitch_m, girder_radius_m, on_surface, bare_albedo)`.
    lattice: vec4<f32>,
    /// Albedo, and work lights.
    girder: vec4<f32>,
    /// `(pitch_m, head_radius_m, offset_m, albedo)`.
    bolts: vec4<f32>,
    /// One bit a region.
    bolted: u32,
    emitted: array<vec4<f32>, REGIONS>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: HullUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var albedo_tiles: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var tile_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var light_tiles: texture_2d_array<f32>;

/// em_render::hull_material::ALL_PLATED.
const ALL_PLATED: f32 = 1.0e30;

/// Rec. 709, matching lc-client's tonemap.
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    out.ship_position = vertex.position;
    out.ship_normal = vertex.normal;
#ifdef HULL_GIRDER
    out.girder = vertex.girder;
#else
    out.regions_0 = vertex.regions_0;
    out.regions_1 = vertex.regions_1;
    out.regions_2 = vertex.regions_2;
    out.regions_3 = vertex.regions_3;
    out.seam = vertex.seam;
#endif
    return out;
}

/// Meters from the joint to `p`, no farther than the span.
fn swept(p: vec3<f32>) -> f32 {
    return min(distance(p, material.reveal.xyz), material.build_fronts.w);
}

/// A band's share at `x` meters out.
fn band(front: f32, width: f32, x: f32) -> f32 {
    return clamp((front - x) / max(width, 1.0e-6), 0.0, 1.0);
}

/// Lit albedo plus what glows, through the exposure.
fn shade(albedo: vec3<f32>, emitted: vec3<f32>, world_normal: vec3<f32>) -> vec4<f32> {
    let to_star = normalize(material.to_star.xyz);
    let lambert = max(dot(normalize(world_normal), to_star), 0.0);
    let light = max(lambert, material.to_star.w);
    let linear = material.reflected.rgb * albedo * light + emitted;

    let reference = material.exposure.x;
    let stops = material.exposure.y;
    let luminance = dot(linear, LUMA);
    let peak = max(linear.r, max(linear.g, linear.b));
    var value = 0.0;
    if (luminance > 0.0 && reference > 0.0 && stops > 0.0) {
        value = clamp(log2(luminance / reference) / stops + 1.0, 0.0, 1.0);
    }
    let chroma = select(vec3<f32>(1.0), linear / peak, peak > 0.0);
    return vec4<f32>(chroma * value, 1.0);
}

fn girder_light() -> vec3<f32> {
    return material.girder.rgb * material.girder.w;
}

#ifdef HULL_GIRDER
/// A girder stands while its band's share is past its threshold: the truss as it goes up, and
/// the scaffold until it comes down.
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let x = min(in.girder.x, material.build_fronts.w);
    let truss = band(material.build_fronts.x, material.build_widths.x, x);
    let down = band(material.build_fronts.z, material.build_widths.z, x);
    let standing = select(truss, truss * (1.0 - down), in.girder.z > 0.5);
    if (material.reveal.w >= ALL_PLATED || standing <= in.girder.y) {
        discard;
    }
    return shade(material.girder.rgb, girder_light(), in.world_normal);
}
#else

struct Texel {
    albedo: vec3<f32>,
    lit: f32,
}

/// The three projections' weights. Sharp, so two tiles overlap only on a narrow band.
fn planes(n: vec3<f32>) -> vec3<f32> {
    let a = pow(abs(normalize(n)), vec3<f32>(8.0));
    return a / (a.x + a.y + a.z);
}

/// Region `layer`'s tile at ship position `p`, mapped triplanar. Each plane's `v` is along the
/// ship's y where it can be, so window bands run the same way on every side.
fn triplanar(p: vec3<f32>, w: vec3<f32>, layer: u32) -> Texel {
    let s = p / material.detail.x;
    let on_x = s.zy;
    let on_y = s.xz;
    let on_z = s.xy;
    let albedo = textureSample(albedo_tiles, tile_sampler, on_x, layer).rgb * w.x
        + textureSample(albedo_tiles, tile_sampler, on_y, layer).rgb * w.y
        + textureSample(albedo_tiles, tile_sampler, on_z, layer).rgb * w.z;
    let lit = textureSample(light_tiles, tile_sampler, on_x, layer).r * w.x
        + textureSample(light_tiles, tile_sampler, on_y, layer).r * w.y
        + textureSample(light_tiles, tile_sampler, on_z, layer).r * w.z;
    return Texel(albedo, lit);
}

fn hash(cell: vec3<f32>) -> f32 {
    let q = fract(cell * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yxz + 33.33);
    return fract((r.x + r.y) * r.z);
}

/// The axis the normal is nearest, as a one; the other two are the face's.
fn dominant(n: vec3<f32>) -> vec3<f32> {
    let a = abs(n);
    var keep = vec3<f32>(0.0);
    if (a.x >= a.y && a.x >= a.z) {
        keep.x = 1.0;
    } else if (a.y >= a.z) {
        keep.y = 1.0;
    } else {
        keep.z = 1.0;
    }
    return keep;
}

/// Whether the plating has reached the panel at `p`. Panels are cells of the dominant
/// projection, each with its own hashed lag, so the front arrives as panels, not as a line.
fn plated(p: vec3<f32>, n: vec3<f32>) -> bool {
    let front = material.reveal.w;
    if (front >= ALL_PLATED) {
        return true;
    }
    let panel_m = material.reveal_panel.x;
    // The dominant axis is left alone, so the cell is a column through the hull and its
    // center stays on the surface.
    let keep = dominant(n);
    let cell = floor(p / panel_m) * (1.0 - keep) + keep * 0.5;
    let center = mix((cell + 0.5) * panel_m, p, keep);
    // Signed, so the two faces of a thin part lag independently.
    let lag = hash(cell + keep * 17.0 * sign(n)) * material.reveal_panel.y;
    return swept(center) + lag <= front;
}

/// The lattice where it crosses a face: girders along the face's two axes at every pitch.
struct Lines {
    /// How much of the fragment is girder, and the same as a mean over the lattice.
    sharp: f32,
    mean: f32,
    /// The nearest girder's.
    threshold: f32,
    /// 1 once a girder is under a pixel, where `mean` is what to draw.
    under: f32,
}

fn lattice_lines(p: vec3<f32>, n: vec3<f32>, pixel: f32) -> Lines {
    let pitch = material.lattice.x;
    let r = material.lattice.y;
    let keep = dominant(n);
    let q = p / pitch;
    let off = abs(fract(q + 0.5) - 0.5) * pitch + keep * 1.0e9;
    let d = min(off.x, min(off.y, off.z));
    // The plane the nearest girder lies in names it, with the segment along the girder and the
    // layer through the face.
    var across = vec3<f32>(0.0);
    if (off.x == d) {
        across.x = 1.0;
    } else if (off.y == d) {
        across.y = 1.0;
    } else {
        across.z = 1.0;
    }
    let id = mix(floor(q), round(q), across + keep) + across * 0.37;
    let sharp = 1.0 - smoothstep(r - 0.5 * pixel, r + 0.5 * pixel, d);
    let c = min(2.0 * r / pitch, 1.0);
    return Lines(sharp, 2.0 * c - c * c, hash(id), smoothstep(r, 3.0 * r, pixel));
}

/// Meters a pixel spans on the surface.
fn pixel_m(p: vec3<f32>) -> f32 {
    return max(length(fwidth(p)), 1.0e-4);
}

/// The two heaviest regions at a point, and the second's share of the pair.
struct Pair {
    first: u32,
    second: u32,
    share: f32,
}

fn heaviest(in: VertexOutput) -> Pair {
    let weights = array<vec4<f32>, 4>(in.regions_0, in.regions_1, in.regions_2, in.regions_3);
    var first = 0u;
    var second = 0u;
    var w1 = -1.0;
    var w2 = -1.0;
    for (var i = 0u; i < REGIONS; i++) {
        let w = weights[i / 4u][i % 4u];
        if (w > w1) {
            second = first;
            w2 = w1;
            first = i;
            w1 = w;
        } else if (w > w2) {
            second = i;
            w2 = w;
        }
    }
    return Pair(first, second, max(w2, 0.0) / max(w1 + max(w2, 0.0), 1.0e-6));
}

/// How much of the fragment is a bolt head, from its seam coordinates. A row too fine to see is
/// drawn as its share of the pixels it crosses, so it fades rather than sparkling.
fn bolt_cover(seam: vec2<f32>) -> f32 {
    let pitch = material.bolts.x;
    let r = material.bolts.y;
    let across = abs(seam.x) - material.bolts.z;
    let along = (fract(seam.y / pitch) - 0.5) * pitch;
    // Meters a pixel spans.
    let w = max(fwidth(seam.x) + fwidth(seam.y), 1.0e-4);
    let d = length(vec2<f32>(across, along));
    let sharp = 1.0 - smoothstep(r - 0.5 * w, r + 0.5 * w, d);
    let wide = max(w, 2.0 * r);
    let band = 1.0 - smoothstep(0.25 * wide, 0.75 * wide, abs(across));
    let mean = min(3.14159265 * r * r / (pitch * wide), 1.0) * band;
    return mix(sharp, mean, smoothstep(r, 3.0 * r, w));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let pair = heaviest(in);
    let w = planes(in.ship_normal);
    let near = triplanar(in.ship_position, w, pair.first);
    let far = triplanar(in.ship_position, w, pair.second);
    // Before any discard: these take derivatives.
    let bolt = bolt_cover(in.seam);
    let pixel = pixel_m(in.ship_position);
    let lines = lattice_lines(in.ship_position, in.ship_normal, pixel);
    let plated_here = plated(in.ship_position, in.ship_normal);
    let t = pair.share;
    var albedo = mix(near.albedo, far.albedo, t);
    let side = select(max(pair.first, pair.second), min(pair.first, pair.second), in.seam.x > 0.0);
    if (((material.bolted >> side) & 1u) != 0u) {
        albedo = mix(albedo, vec3<f32>(material.bolts.w), bolt);
    }
    var emitted = mix(
        near.lit * material.emitted[pair.first].rgb,
        far.lit * material.emitted[pair.second].rgb,
        t,
    );
    if (material.reveal.w >= ALL_PLATED) {
        return shade(albedo, emitted, in.world_normal);
    }

    let x = swept(in.ship_position);
    let fitted = band(material.build_fronts.y, material.build_widths.y, x);
    let fitted_albedo = albedo;
    albedo = mix(vec3<f32>(material.lattice.w), albedo, fitted);
    emitted *= fitted;
    if (material.lattice.z < 0.5) {
        // A girder mesh stands behind the gaps.
        if (!plated_here) {
            discard;
        }
        return shade(albedo, emitted, in.world_normal);
    }

    // No girder mesh: the lattice is drawn here, in place of plating not yet up and over plating
    // as scaffold. Once a girder is under a pixel nothing is discarded, and plating, gaps and
    // girders are drawn as their shares of the pixel.
    let truss = band(material.build_fronts.x, material.build_widths.x, x);
    let scaffold = truss * (1.0 - band(material.build_fronts.z, material.build_widths.z, x));
    // Seen from afar, a line of sight crosses every layer of truss the sliver holds.
    let through = 1.0 - pow(1.0 - lines.mean * truss, max(material.build_widths.w, 1.0));
    let open = mix(select(0.0, lines.sharp, truss > lines.threshold), through, lines.under);
    let over = mix(select(0.0, lines.sharp, scaffold > lines.threshold), lines.mean * scaffold, lines.under);
    let panel_far = smoothstep(0.3, 1.0, pixel / material.reveal_panel.x);
    let smooth_plating = band(material.reveal.w, material.reveal_panel.y, x);
    let plating = mix(select(0.0, 1.0, plated_here), smooth_plating, max(panel_far, lines.under));
    // Where nothing is up yet there is nothing to draw. Between girders too far off to resolve,
    // what shows is taken to be the part's own material, which for a part growing is the old
    // hull just behind.
    let bare = lines.under < 0.5 && open < 0.5;
    if (plating < 0.5 && (bare || truss <= 0.0)) {
        discard;
    }
    let girder = material.girder.rgb;
    let open_albedo = mix(fitted_albedo, girder, open);
    let closed_albedo = mix(albedo, girder, over);
    let open_emitted = girder_light() * open;
    let closed_emitted = mix(emitted, girder_light(), over);
    return shade(mix(open_albedo, closed_albedo, plating), mix(open_emitted, closed_emitted, plating), in.world_normal);
}
#endif
