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
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}
#import "shaders/scars.wgsl"::{glass_specular, knapped}
#import "shaders/panels.wgsl"::{Plate, plate}
#import "shaders/habitat.wgsl"::{canopy, hex, room, stratus, window}
#import "shaders/hash.wgsl"::{mix_hash, pcg, unit}
#import "shaders/pipes.wgsl"::{PipeTiling, pipe_tiling, tile_start, variant_layer}

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
    // em_render::hull_material::ATTRIBUTE_HULL_FACE: (share, face / 255, 0, 0).
    @location(6) face: vec4<f32>,
    // em_render::hull_material::ATTRIBUTE_HULL_CHART and ATTRIBUTE_HULL_CHART_BOUNDS, and the
    // direction the chart's u grows, in the ship's frame.
    @location(7) chart: vec4<f32>,
    @location(8) bounds: vec4<f32>,
    @location(9) tangent: vec4<f32>,
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
    @location(7) face_share: f32,
    @location(8) @interpolate(flat) face: u32,
    // The camera, and the direction to the star, in the ship's frame: glass is lit where its
    // scars are cut.
    @location(9) eye_ship: vec3<f32>,
    @location(10) star_ship: vec3<f32>,
    @location(11) chart: vec4<f32>,
    @location(12) @interpolate(flat) bounds: vec4<f32>,
    @location(13) chart_tangent: vec3<f32>,
#endif
}

const REGIONS: u32 = 16u;
const FACES: u32 = 16u;

struct HullUniform {
    /// World direction to the star; `w` is the light on the unlit side.
    to_star: vec4<f32>,
    reflected: vec4<f32>,
    /// Even over the hull, in reflected's units.
    glow: vec4<f32>,
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
    /// One bit a region: knapped glass.
    glassy: u32,
    /// One bit a region: azteced plate.
    plated: u32,
    /// One bit a region: habitat.
    habitat: u32,
    /// One bit a layer: layered sprue.
    piped: u32,
    emitted: array<vec4<f32>, REGIONS>,
    /// `(region, flank, on, 0)`.
    open: vec4<u32>,
    faces: array<vec4<f32>, FACES>,
    /// `(scar_m, dish, tilt, ripple)`.
    glass: vec4<f32>,
    /// Color, and roughness in `w`.
    glass_finish: vec4<f32>,
    /// `(cell_m, min_panel_m, snap_m, 0)`.
    plates: vec4<f32>,
    /// `(tilt_rad, brown, chamfer_m, lean_rad)`.
    plate_finish: vec4<f32>,
    /// `(roughness, metallic, paint, 0)`.
    plate_metal: vec4<f32>,
    /// `(deck_m, pitch_m, sill_m, room_m)`.
    deck: vec4<f32>,
    /// `(width_m, height_m, f0, roughness)`.
    pane: vec4<f32>,
    /// `(hex_m, seam_m, rim_m, forest_m)`.
    roof: vec4<f32>,
    /// The air's color by day, and what of the star it scatters back up in `w`.
    roof_sky: vec4<f32>,
    /// `(depth_share, lit_share, 0, 0)`.
    roof_shape: vec4<f32>,
    /// `(layer_1_m, layer_2_m, tile_m, 0)`.
    pipes: vec4<f32>,
    /// `(grid_m, bar_m, cell_m, depth_m)`; no thruster where `grid_m` is 0.
    thruster: vec4<f32>,
    /// The cells' hue, linear.
    thruster_glow: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: HullUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var albedo_tiles: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var tile_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var light_tiles: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var pipe_tiles: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var pipe_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var pipe_glow: texture_2d_array<f32>;

/// Obsidian's index is about 1.49.
const GLASS_F0: f32 = 0.04;
/// The sky glass mirrors, in reflected's units: dim, but enough that a facet shows off the highlight.
const GLASS_SKY: f32 = 0.25;
/// The same for plate metal.
const PLATE_SKY: f32 = 0.1;
const PI: f32 = 3.14159265;
/// The roof's clear glass.
const ROOF_F0: f32 = 0.04;
const ROOF_ROUGHNESS: f32 = 0.05;
/// How far down the stratus lies, as a share of the forest's depth.
const STRATUS_SHARE: f32 = 0.4;
/// What of the forest the thickest stratus hides.
const STRATUS_ALPHA: f32 = 0.6;
/// How far through the dome's air the forest fades to 1/e.
const AIR_M: f32 = 160.0;
const WALL_STEPS: i32 = 32;
/// The sprue's baked light is in units of its lamps' color at a meter; this is that color over the
/// layer's emitted, which is a lit window's face. Floodlights on pipework are far brighter than a
/// window seen from outside, and must outshine the night fill.
const PIPE_LAMP: f32 = 40.0;
/// What of a pipe layer is pipe, on average.
const PIPE_COVER: f32 = 0.35;
/// sixteen variants of three layers, once the sprue is baked; the fallback has one.
const PIPE_LAYERS: u32 = 48u;
const WALL_ALBEDO: f32 = 0.25;

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
    out.face_share = vertex.face.x;
    out.face = u32(round(vertex.face.y * 255.0));
    out.chart = vertex.chart;
    out.bounds = vertex.bounds;
    out.chart_tangent = vertex.tangent.xyz;
    // A ship's transform is a rotation and a uniform scale, so its inverse is the transpose over
    // the scale squared.
    let axes = mat3x3<f32>(world_from_local[0].xyz, world_from_local[1].xyz, world_from_local[2].xyz);
    out.eye_ship = transpose(axes) * (view.world_position - world_from_local[3].xyz) / dot(axes[0], axes[0]);
    out.star_ship = transpose(axes) * material.to_star.xyz;
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
    let linear = material.reflected.rgb * albedo * light + emitted + material.glow.rgb;

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

/// The layer a region is drawn with here: an open-faced region's flank off its faces. Chosen
/// rather than blended, so the tile is sampled once; the switch falls on the face's rim, which is
/// a crease in the hull anyway.
fn layer_of(region: u32, share: f32) -> u32 {
    let open = material.open;
    return select(region, open.y, open.z != 0u && region == open.x && share < 0.5);
}

/// What a fully lit texel of `layer` sends, drawn for `region`.
fn light_of(region: u32, layer: u32, in: VertexOutput) -> vec3<f32> {
    let open = material.open;
    if (layer != region || open.z == 0u || region != open.x) {
        return material.emitted[layer].rgb;
    }
    if (in.face >= FACES) {
        return vec3<f32>(0.0);
    }
    return material.faces[in.face].rgb * clamp(in.face_share, 0.0, 1.0);
}

fn glassy(region: u32) -> bool {
    return ((material.glassy >> region) & 1u) != 0u;
}

/// What a glassy region's knapped surface sends toward the eye, in reflected's units: its color
/// lit by the star at each facet's own lean, the star's highlight, and a faint sky mirrored in
/// every facet, brighter toward the star, so a facet reads away from the highlight too.
fn glass_light(in: VertexOutput, pixel: f32) -> vec3<f32> {
    let surface = normalize(in.ship_normal);
    let star = normalize(in.star_ship);
    let k = knapped(in.ship_position, surface, material.glass.x, material.glass.yzw, pixel, 0u);
    let roughness = mix(0.5, material.glass_finish.w, k.keep_fine);
    let e = normalize(in.eye_ship - in.ship_position);
    let lit_side = dot(star, surface) > 0.0;
    let ne = max(dot(k.normal, e), 0.0);
    let fresnel = GLASS_F0 + (1.0 - GLASS_F0) * pow(1.0 - ne, 5.0);
    let r = reflect(-e, k.normal);
    let facing = select(0.0, max(dot(k.normal, star), 0.0), lit_side);
    var light = material.glass_finish.rgb * max(facing, material.to_star.w);
    light += vec3<f32>(GLASS_SKY * fresnel * (0.3 + 0.7 * max(dot(r, star), 0.0)));
    if (lit_side) {
        // reflected is the star's light over pi, as Lambert's albedo takes it; a BRDF takes it whole.
        light += vec3<f32>(3.14159265 * glass_specular(k.normal, star, e, roughness, GLASS_F0));
    }
    return material.reflected.rgb * light;
}

fn panelled(region: u32) -> bool {
    return ((material.plated >> region) & 1u) != 0u;
}

/// The ship's up: a habitat's roof is the end of its part that faces it.
const SHIP_UP: vec3<f32> = vec3<f32>(0.0, 0.0, 1.0);

const RECT: u32 = 0u;
const DISC: u32 = 1u;
const BAND: u32 = 2u;
const RING: u32 = 3u;
const DOME: u32 = 4u;

/// No window nearer a chart's edge than this, as looks::aztec::Windows::chart_margin_m.
const WINDOW_MARGIN_M: f32 = 1.0;

/// Where a fragment lies in its part's chart (crate::charts in lc-client), as plating and a
/// habitat lay themselves out in it.
struct HullChart {
    /// Meters in the chart, stretched around a band or ring to a whole number of plate cells, which
    /// are whole numbers of snaps, window columns and decks, so every lattice meets itself there.
    p: vec2<f32>,
    /// How far `p`'s u and v run before they repeat; 0 where they do not.
    around: vec2<f32>,
    kind: u32,
    bounds: vec4<f32>,
    /// u, v, and the surface's normal, in the ship's frame.
    frame: mat3x3<f32>,
    /// Alike either side of a seam; a rectangle, all of one direction, told apart by which way its
    /// u runs, as a cube's faces are alike in extent.
    id: u32,
    end: bool,
}

fn hull_chart(in: VertexOutput, surface: vec3<f32>) -> HullChart {
    let kind = u32(round(in.chart.z));
    let u = normalize(in.chart_tangent - surface * dot(in.chart_tangent, surface));
    let frame = mat3x3<f32>(u, cross(surface, u), surface);
    var p = in.chart.xy;
    var around = vec2<f32>(0.0);
    let cell = material.plates.x;
    if (kind == BAND || kind == RING) {
        let n = max(round(in.bounds.x / cell), 1.0);
        p.x *= n * cell / in.bounds.x;
        around.x = n * cell;
    }
    if (kind == RING) {
        let n = max(round(in.bounds.y / cell), 1.0);
        p.y *= n * cell / in.bounds.y;
        around.y = n * cell;
    }
    let facing = select(0u, u32(round(dot(u, vec3<f32>(1.0, 3.0, 9.0))) + 13.0), kind == RECT);
    let id = mix_hash(kind + 8u * facing, u32(in.chart.w) + 2u * bitcast<u32>(in.bounds.x));
    return HullChart(p, around, kind, in.bounds, frame, id, in.chart.w > 0.5);
}

/// What a plated region sends toward the eye, in reflected's units: each panel's own shade lit at
/// its own tilt, a seam's walls at theirs, the star's highlight off the metal, and a faint sky.
fn plate_light(in: VertexOutput, c: HullChart, pixel: f32) -> vec3<f32> {
    let surface = normalize(in.ship_normal);
    let k = plate(c.p, c.frame, c.id, c.around, material.plates, material.plate_finish, pixel, 0u);
    return material.reflected.rgb * plate_lit(k, normalize(in.star_ship), normalize(in.eye_ship - in.ship_position), surface);
}

/// A plate's light in units of reflected: each panel's own shade lit at its own tilt, a seam's walls
/// at theirs, the star's highlight off the metal, and a faint sky.
fn plate_lit(k: Plate, star: vec3<f32>, e: vec3<f32>, surface: vec3<f32>) -> vec3<f32> {
    let lit_side = dot(star, surface) > 0.0;
    let roughness = material.plate_metal.x;
    let metallic = material.plate_metal.y;
    let color = k.color * material.plate_metal.z;
    let albedo = color * (1.0 - metallic);
    let luma = max(dot(color, LUMA), 1.0e-3);
    let f0 = mix(0.04, luma, metallic);
    let tint = mix(vec3<f32>(1.0), color / luma, metallic);
    var light = vec3<f32>(0.0);
    for (var i = 0; i < 2; i++) {
        let n = select(k.normal, k.wall, i == 1);
        light += metal_lit(n, star, e, lit_side, albedo, f0, tint, roughness) * select(1.0 - k.wall_share, k.wall_share, i == 1);
    }
    return light;
}

fn metal_lit(n: vec3<f32>, star: vec3<f32>, e: vec3<f32>, lit_side: bool, albedo: vec3<f32>, f0: f32, tint: vec3<f32>, roughness: f32) -> vec3<f32> {
    let facing = select(0.0, max(dot(n, star), 0.0), lit_side);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - max(dot(n, e), 0.0), 5.0);
    let r = reflect(-e, n);
    var here = albedo * max(facing, material.to_star.w);
    here += tint * PLATE_SKY * fresnel * (0.3 + 0.7 * max(dot(r, star), 0.0));
    if (lit_side) {
        here += tint * PI * glass_specular(n, star, e, roughness, f0);
    }
    return here;
}

/// Whether `layer` drawn for `region` is an open face drawn as a thruster.
fn thrusting(region: u32, layer: u32) -> bool {
    let open = material.open;
    return material.thruster.x > 0.0 && open.z != 0u && region == open.x && layer == region;
}

/// An engine's open face as an ion thruster's, in reflected's units: a uniform grid of dark bars
/// over hex cells a little way behind it, seen through its gaps at their depth, so they move
/// against it as the eye does. Each cell glows at what leaves the face, `face`, in the thruster's
/// hue, brightest and palest in its middle, between dark walls. Bars and cells smaller than a pixel are drawn
/// as their averages.
fn thruster_light(in: VertexOutput, pixel: f32, face: vec3<f32>) -> vec3<f32> {
    let surface = normalize(in.ship_normal);
    let star = normalize(in.star_ship);
    let e = normalize(in.eye_ship - in.ship_position);
    // The face is flat, so its own normal names a frame on it.
    let across = select(SHIP_UP, vec3<f32>(1.0, 0.0, 0.0), abs(surface.z) > 0.9);
    let u = normalize(cross(across, surface));
    let v = cross(surface, u);
    let p = vec2<f32>(dot(in.ship_position, u), dot(in.ship_position, v));
    let grid = material.thruster.x;
    let bar = material.thruster.y;
    let cell = material.thruster.z;

    let to_bar = abs(fract(p / grid + 0.5) - 0.5) * grid;
    let near_bar = 1.0 - smoothstep(0.5 * bar - pixel, 0.5 * bar + pixel, min(to_bar.x, to_bar.y));
    let open_share = 1.0 - bar / grid;
    let bars = mix(near_bar, 1.0 - open_share * open_share, smoothstep(0.25, 1.0, pixel / grid));

    let d = vec3<f32>(-dot(e, u), -dot(e, v), max(dot(e, surface), 0.05));
    let q = p + d.xy * material.thruster.w / d.z;
    let h = hex(q, cell);
    let wall = 0.08 * cell;
    let middle = clamp((h.z - wall) / (0.5 * cell - wall), 0.0, 1.0);
    let lit_cell = smoothstep(wall - pixel, wall + pixel, h.z) * (0.25 + 0.75 * sqrt(middle));
    let cell_id = mix_hash(u32(i32(round(4.0 * (q.x - h.x)))), u32(i32(round(4.0 * (q.y - h.y)))));
    let flicker = mix(0.85, 1.15, unit(pcg(cell_id)));
    let profile = mix(lit_cell * flicker, 0.55, smoothstep(0.25, 1.0, pixel / cell));
    // Deep at a cell's walls, paling toward its core, as a plasma's glow does.
    let deep = material.thruster_glow.rgb / max(dot(material.thruster_glow.rgb, LUMA), 1.0e-3);
    let core = mix(material.thruster_glow.rgb, vec3<f32>(1.0), 0.55);
    let hue = mix(deep, core / dot(core, LUMA), middle * middle);
    let glow = hue * dot(face, LUMA) * profile;

    let lit_side = dot(star, surface) > 0.0;
    let back = material.reflected.rgb * metal_lit(surface, star, e, lit_side, vec3<f32>(0.03), 0.04, vec3<f32>(1.0), 0.6);
    let metal = material.reflected.rgb * metal_lit(surface, star, e, lit_side, vec3<f32>(0.06), 0.25, vec3<f32>(1.0), 0.4);
    // The bars are lit from behind too, by the cells around them.
    let bar_light = metal + 0.08 * deep * dot(face, LUMA);
    return mix(glow + back, bar_light, bars);
}

fn piped(layer: u32) -> bool {
    return ((material.piped >> layer) & 1u) != 0u;
}

fn pipe_height(k: u32) -> f32 {
    return select(select(0.0, material.pipes.x, k == 1u), material.pipes.y, k == 2u);
}

/// What of the star reaches layer `k` at chart point `q`, past the cutout layers above it.
fn pipe_starlit(q: vec2<f32>, k: u32, toward: vec3<f32>, id: f32, tiling: PipeTiling) -> f32 {
    var through = 1.0;
    for (var above = k + 1u; above <= 2u; above++) {
        let at = q + toward.xy * (pipe_height(above) - pipe_height(k)) / toward.z;
        let uv = at / tiling.size;
        let cover = textureSampleLevel(pipe_tiles, pipe_sampler, fract(uv), variant_layer(uv, above, id, tiling.around), 1.0).a;
        through *= 1.0 - smoothstep(0.35, 0.65, cover);
    }
    return through;
}

/// What a layered sprue sends toward the eye, in reflected's units. The surface is the top layer
/// of pipe; through its gaps the layer under it and the plated base are seen at their depths, so
/// they move against it as the eye does. Each is lit by its baked lamps in `lamp`'s color, and by
/// the star past the layers above. Laid out in the part's chart: `p` is the point there, in meters,
/// `tiling` how the tiles fit it, and `frame` its u, v and the surface's normal. `ddx` and `ddy`
/// are the tile coordinates' derivatives, taken outside any branch.
fn pipe_lit(in: VertexOutput, p: vec2<f32>, tiling: PipeTiling, id: f32, frame: mat3x3<f32>, ddx: vec2<f32>, ddy: vec2<f32>, lamp: vec3<f32>) -> vec3<f32> {
    let star = normalize(in.star_ship);
    let e = normalize(in.eye_ship - in.ship_position);
    let d = vec3<f32>(-dot(e, frame[0]), -dot(e, frame[1]), max(dot(e, frame[2]), 0.05));
    let toward = vec3<f32>(dot(star, frame[0]), dot(star, frame[1]), dot(star, frame[2]));
    // Front to back, each layer over what is under it by its coverage: 0 or 1 where a texel is
    // resolved, its share where a mip averages pipe and gap, so a distant flank does not flicker
    // between layers pixel to pixel.
    // The tiles' mips keep each layer's coverage by keeping alpha near 0 or 1, right for a cutout
    // and wrong for blending, so where a pixel spans several texels the layer's mean share stands in.
    let texels = max(length(ddx), length(ddy)) * f32(textureDimensions(pipe_tiles).x);
    let far = smoothstep(0.75, 3.0, texels);
    var light = vec3<f32>(0.0);
    var through = 1.0;
    for (var i = 0u; i < 3u; i++) {
        let k = 2u - i;
        let q = p + d.xy * (pipe_height(2u) - pipe_height(k)) / d.z;
        let uv = q / tiling.size;
        let layer = variant_layer(uv, k, id, tiling.around);
        let texel = textureSampleGrad(pipe_tiles, pipe_sampler, fract(uv), layer, ddx, ddy);
        let cover = select(mix(texel.a, PIPE_COVER, far), 1.0, k == 0u);
        if (cover > 0.0) {
            let baked = textureSampleGrad(pipe_glow, pipe_sampler, fract(uv), layer, ddx, ddy).rgb;
            let tn = texel.rg * 2.0 - 1.0;
            let n = normalize(frame * vec3<f32>(tn, sqrt(max(1.0 - dot(tn, tn), 0.0))));
            let albedo = texel.b;
            var lit = vec3<f32>(albedo * material.to_star.w);
            if (toward.z > 0.0) {
                let spec = 0.35 * pow(max(dot(n, normalize(star + e)), 0.0), 24.0);
                lit += vec3<f32>((albedo + spec) * max(dot(n, star), 0.0) * pipe_starlit(q, k, toward, id, tiling));
            }
            light += through * cover * (material.reflected.rgb * lit + lamp * PIPE_LAMP * (baked + 0.015 * albedo));
        }
        through *= 1.0 - cover;
    }
    return light;
}

fn habitable(region: u32) -> bool {
    return ((material.habitat >> region) & 1u) != 0u;
}

/// Signed meters from `q`, in the roof's chart, to its edge, positive inside.
fn inside_roof(q: vec2<f32>, c: HullChart) -> f32 {
    if (c.kind == RECT) {
        let d = min(q - c.bounds.xz, c.bounds.yw - q);
        return min(d.x, d.y);
    }
    if (c.kind == DISC || c.kind == DOME) {
        return c.bounds.x - length(q);
    }
    return 1.0e6;
}

/// The roof chart's width: what the forest's depth is a share of.
fn roof_width(c: HullChart) -> f32 {
    if (c.kind == RECT) {
        return min(c.bounds.y - c.bounds.x, c.bounds.w - c.bounds.z);
    }
    if (c.kind == DISC || c.kind == DOME) {
        return 2.0 * c.bounds.x;
    }
    return 0.0;
}

/// How far down the eye's ray through the glass, going `d` in the roof's chart with z down, meets
/// the wall dropping from the rim, or `limit` if it reaches that depth first. Walls drop square to
/// the chart, so the roof's own edge is a safe step along the ray's shadow on it.
fn wall_depth(c: HullChart, d: vec3<f32>, limit: f32) -> f32 {
    let slope = d.xy / d.z;
    let run = max(length(slope), 1.0e-3);
    var h = 0.0;
    for (var i = 0; i < WALL_STEPS; i++) {
        let inside = inside_roof(c.p + slope * h, c);
        if (i > 0 && inside < 0.02) {
            return h;
        }
        h += max(inside, 0.05) / run;
        if (h >= limit) {
            return limit;
        }
    }
    return limit;
}

/// What a roof pane shows, in units of reflected: no light of its own, only the star's let in
/// through the glass. A forest a share of the roof's width down, at most roof.w, is seen where the
/// eye's ray meets it, so it moves against the frame, through air that scatters the star back blue
/// by day and red as it sets over the roof and hides the forest the longer the ray's path; walls
/// drop from the rim to the forest; a stratus deck part way down lies over it and shades it.
fn under_roof(c: HullChart, surface: vec3<f32>, e: vec3<f32>, star: vec3<f32>, edge: f32) -> vec3<f32> {
    let p = c.p;
    let over = dot(star, surface);
    let day = smoothstep(-0.08, 0.25, over);
    let high = smoothstep(0.0, 0.45, over);
    let tint = mix(vec3<f32>(1.0, 0.42, 0.16), material.roof_sky.rgb, high);
    let rayleigh = 0.75 * (1.0 + pow(dot(-star, e), 2.0));
    // The star is pi in reflected's units.
    let air = PI * material.roof_sky.w * day * mix(vec3<f32>(1.0), tint, 0.7) * rayleigh;

    let width = roof_width(c);
    let floor_m = select(material.roof.w, min(material.roof.w, material.roof_shape.x * width), width > 0.0);
    let deck_m = STRATUS_SHARE * floor_m;
    let d = vec3<f32>(-dot(e, c.frame[0]), -dot(e, c.frame[1]), max(dot(e, c.frame[2]), 0.02));
    let sunlight = day * mix(vec3<f32>(1.0, 0.6, 0.35), vec3<f32>(1.0), high);
    let star_face = vec3<f32>(dot(star, c.frame[0]), dot(star, c.frame[1]), dot(star, c.frame[2]));
    let depth = wall_depth(c, d, floor_m);
    let through = exp(-(depth / d.z) / AIR_M);
    var seen: vec3<f32>;
    if (depth < floor_m) {
        let band = select(1.0, 0.5, fract(depth / max(material.deck.x, 1.0)) < 0.06);
        seen = sunlight * WALL_ALBEDO * (0.3 + 0.7 * max(over, 0.0)) * mix(1.0, 0.4, depth / floor_m) * band;
    } else {
        let ground = p + d.xy * (floor_m / d.z);
        let shade = 1.0 - 0.6 * stratus(ground + star_face.xy * (floor_m - deck_m) / max(star_face.z, 0.1));
        seen = canopy(ground, star_face) * sunlight * shade;
    }
    var below = seen * through + air * (1.0 - through);
    if (depth > deck_m) {
        let deck_path = deck_m / d.z;
        let cover = STRATUS_ALPHA * stratus(p + d.xy * deck_path);
        let deck_through = exp(-deck_path / AIR_M);
        let cloud = PI * sunlight * (0.2 + 0.3 * max(over, 0.0)) * mix(0.8, 1.0, high);
        below = mix(below, cloud * deck_through + air * (1.0 - deck_through), cover);
    }
    // The frame shades what is just inside it.
    return below * mix(0.55, 1.0, smoothstep(0.0, 2.0, edge));
}

/// What a habitat region sends toward the eye, in reflected's units: office plate on the sides with
/// windows over traced rooms, lit by `glow` where a room's lights are on, and over the top a roof of
/// glass hexes on a forest, chamfered where the two meet. The roof is the part's end that faces
/// the ship's up; the sides' windows keep clear of their charts' edges.
fn habitat_light(in: VertexOutput, c: HullChart, pixel: f32, glow: vec3<f32>) -> vec3<f32> {
    let surface = normalize(in.ship_normal);
    let star = normalize(in.star_ship);
    let e = normalize(in.eye_ship - in.ship_position);
    let lit_side = dot(star, surface) > 0.0;
    let metal = vec3<f32>(0.4) * material.plate_metal.z;
    let metal_f0 = mix(0.04, dot(metal, LUMA), material.plate_metal.y);
    var lit = vec3<f32>(0.0);
    var lamp = vec3<f32>(0.0);
    let roofed = c.end && surface.z > 0.0;
    if (roofed) {
        let cell = hex(c.p, material.roof.x);
        let frame = metal_lit(surface, star, e, lit_side, metal * (1.0 - material.plate_metal.y), metal_f0, vec3<f32>(1.0), material.plate_metal.x);
        let fresnel = ROOF_F0 + (1.0 - ROOF_F0) * pow(1.0 - max(dot(surface, e), 0.0), 5.0);
        let half_seam = 0.5 * material.roof.y;
        var pane = (1.0 - fresnel) * under_roof(c, surface, e, star, cell.z - half_seam);
        pane += vec3<f32>(PLATE_SKY * fresnel);
        if (lit_side) {
            pane += vec3<f32>(PI * glass_specular(surface, star, e, ROOF_ROUGHNESS, ROOF_F0));
        }
        lit = mix(frame, pane, smoothstep(half_seam - pixel, half_seam + pixel, cell.z));
    } else {
        let k = plate(c.p, c.frame, c.id, c.around, material.plates, material.plate_finish, pixel, 0u);
        lit = plate_lit(k, star, e, surface);
        // On the sides, clear of the chart's edges; glazing leans no more than 30 degrees off the
        // vertical.
        var lo = vec2<f32>(-1.0e9);
        var hi = vec2<f32>(1.0e9);
        if (c.kind == RECT) {
            lo = c.bounds.xz + WINDOW_MARGIN_M;
            hi = c.bounds.yw - WINDOW_MARGIN_M;
        } else if (c.kind == BAND) {
            lo.y = c.bounds.z + WINDOW_MARGIN_M;
            hi.y = c.bounds.w - WINDOW_MARGIN_M;
        }
        if (!c.end && abs(surface.z) <= 0.5) {
            let clear = material.plate_finish.z + 0.1;
            let pane = window(c.p, k.panel, c.id, material.deck, material.pane.xy, clear, lo, hi, material.roof_shape.y, c.around, vec2<f32>(pixel));
            if (pane.cover > 0.0) {
                let fresnel = material.pane.z + (1.0 - material.pane.z) * pow(1.0 - max(dot(k.normal, e), 0.0), 5.0);
                var glass = vec3<f32>(PLATE_SKY * fresnel);
                if (lit_side) {
                    glass += vec3<f32>(PI * glass_specular(k.normal, star, e, material.pane.w, material.pane.z));
                }
                let on = select(0.0, 1.0, pane.lit);
                lamp = pane.cover * (1.0 - fresnel) * on * room(c.p, e, c.frame, c.id, pane, material.deck, material.deck.w);
                lit = mix(lit, glass, pane.cover);
            }
        }
    }

    // The chamfer where the roof meets the sides: each side of the line leans 45 degrees toward it.
    // On the roof, toward its chart's edge; on a side, toward the edge of its chart that lies up.
    var dist = 1.0e6;
    var out = vec2<f32>(0.0);
    if (roofed) {
        if (c.kind == RECT) {
            let to_edge = vec4<f32>(c.p - c.bounds.xz, c.bounds.yw - c.p);
            dist = min(min(to_edge.x, to_edge.y), min(to_edge.z, to_edge.w));
            out = vec2<f32>(0.0, 1.0);
            if (dist == to_edge.x) { out = vec2<f32>(-1.0, 0.0); }
            else if (dist == to_edge.y) { out = vec2<f32>(0.0, -1.0); }
            else if (dist == to_edge.z) { out = vec2<f32>(1.0, 0.0); }
        } else if (c.kind == DISC || c.kind == DOME) {
            dist = c.bounds.x - length(c.p);
            out = c.p / max(length(c.p), 1.0e-4);
        }
    } else if (c.kind == RECT || c.kind == BAND) {
        let up = dot(c.frame[1], SHIP_UP) >= 0.0;
        dist = select(c.p.y - c.bounds.z, c.bounds.w - c.p.y, up);
        out = vec2<f32>(0.0, select(-1.0, 1.0, up));
    }
    let toward = c.frame[0] * out.x + c.frame[1] * out.y;
    let half_width = 0.5 * material.roof.z;
    let share = (1.0 - smoothstep(half_width - pixel, half_width + pixel, dist)) * clamp(half_width / pixel, 0.0, 1.0);
    let bevel = metal_lit(normalize(surface + toward), star, e, lit_side, metal * (1.0 - material.plate_metal.y), metal_f0, vec3<f32>(1.0), material.plate_metal.x);
    return material.reflected.rgb * mix(lit, bevel, share) + glow * lamp * (1.0 - share);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let pair = heaviest(in);
    let w = planes(in.ship_normal);
    let near_layer = layer_of(pair.first, in.face_share);
    let far_layer = layer_of(pair.second, in.face_share);
    let near = triplanar(in.ship_position, w, near_layer);
    let far = triplanar(in.ship_position, w, far_layer);
    // Before any discard: these take derivatives.
    let pixel = pixel_m(in.ship_position);
    // Taken here, outside any branch, for the habitat's rim chamfer.
    let up_normal = normalize(in.ship_normal);
    let chart = hull_chart(in, up_normal);
    // The sprue in the part's chart, a whole number of tiles around a wrap, each chart starting its
    // tiles somewhere else; a rectangle, all of one direction, told apart by which way its u runs,
    // as a cube's faces are alike in extent.
    let pipe_tiles_fit = pipe_tiling(material.pipes.z, chart.kind, in.bounds);
    let pipe_frame = chart.frame;
    let facing = select(0.0, round(dot(chart.frame[0], vec3<f32>(1.0, 3.0, 9.0))), chart.kind == RECT);
    let pipe_id = f32(chart.kind) * 31.0 + in.chart.w * 7.0 + facing;
    let pipe_p = in.chart.xy + tile_start(pipe_id) * pipe_tiles_fit.size;
    let pipe_uv = pipe_p / pipe_tiles_fit.size;
    let pipe_dx = dpdx(pipe_uv);
    let pipe_dy = dpdy(pipe_uv);
    let lines = lattice_lines(in.ship_position, in.ship_normal, pixel);
    let plated_here = plated(in.ship_position, in.ship_normal);
    let t = pair.share;
    var albedo = mix(near.albedo, far.albedo, t);
    var emitted = mix(
        near.lit * light_of(pair.first, near_layer, in),
        far.lit * light_of(pair.second, far_layer, in),
        t,
    );
    let glass = select(0.0, 1.0 - t, glassy(pair.first)) + select(0.0, t, glassy(pair.second));
    if (glass > 0.0) {
        // Glass is lit at its facets' lean in glass_light, not as albedo at the surface's.
        albedo *= 1.0 - glass;
        emitted += glass * glass_light(in, pixel);
    }
    let home = select(0.0, 1.0 - t, habitable(pair.first)) + select(0.0, t, habitable(pair.second));
    if (home > 0.0) {
        let living = select(pair.second, pair.first, habitable(pair.first));
        albedo *= 1.0 - home;
        emitted = emitted * (1.0 - home) + home * habitat_light(in, chart, pixel, material.emitted[living].rgb);
    }
    let thrust_share = select(0.0, 1.0 - t, thrusting(pair.first, near_layer)) + select(0.0, t, thrusting(pair.second, far_layer));
    if (thrust_share > 0.0) {
        let face = select(vec3<f32>(0.0), material.faces[min(in.face, FACES - 1u)].rgb, in.face < FACES);
        albedo *= 1.0 - thrust_share;
        emitted = emitted * (1.0 - thrust_share) + thrust_share * thruster_light(in, pixel, face * clamp(in.face_share, 0.0, 1.0));
    }
    let pipe_share = select(0.0, 1.0 - t, piped(near_layer)) + select(0.0, t, piped(far_layer));
    if (pipe_share > 0.0 && textureNumLayers(pipe_tiles) >= PIPE_LAYERS) {
        let flank = select(far_layer, near_layer, piped(near_layer));
        albedo *= 1.0 - pipe_share;
        emitted = emitted * (1.0 - pipe_share) + pipe_share * pipe_lit(in, pipe_p, pipe_tiles_fit, pipe_id, pipe_frame, pipe_dx, pipe_dy, material.emitted[flank].rgb);
    }
    let plate_share = select(0.0, 1.0 - t, panelled(pair.first)) + select(0.0, t, panelled(pair.second));
    if (plate_share > 0.0) {
        albedo *= 1.0 - plate_share;
        emitted += plate_share * plate_light(in, chart, pixel);
    }
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
