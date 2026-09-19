//! The sky as geometry: one quad per star, fed by [`crate::session::Session`].
//!
//! The material and the shader live in `em-render`. This is the half that cannot be shared —
//! building the mesh, generating the band table, and keeping the uniforms current.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_mesh::{Indices, PrimitiveTopology};
use em_render::relativistic_starfield_material::{
    ATTRIBUTE_STAR_CORNER, ATTRIBUTE_STAR_PARAMS, ATTRIBUTE_STAR_WARM, BANDS,
    RelativisticStarfieldMaterial, RelativisticStarfieldUniform,
};
use em_render::render_space::sim_to_render;
use em_spectra::{Band, BandMapping, blackbody};
use glam::DVec3;
use lc_world::rng;
use lc_world::sky::{CatalogueStar, generate};

use crate::session::{POINT_STOPS, Session};

/// Temperature samples in the band table, over [`LOG_T_MIN`] to [`LOG_T_MAX`].
pub const LUT_SAMPLES: usize = 2048;

/// The table's domain, as `log2` of kelvin: 16 K to about 4 million K.
///
/// Far wider than any star, because the shader looks the table up at the *Doppler-shifted*
/// temperature. At the drive's 0.999c cap the shift factor is 44.7 either way, so a 2000 K red
/// dwarf astern arrives at 45 K and a 50 000 K O star ahead arrives at 2.2 million. Both ends
/// have to be in the table or the sky clips to a flat colour at speed.
pub const LOG_T_MIN: f32 = 4.0;
pub const LOG_T_MAX: f32 = 22.0;

/// How far the ship travels before the mesh is rebuilt about its new position, light-years.
///
/// Star positions are baked relative to a bake origin and the ship's offset from that origin
/// is a uniform, so the shader differences two small numbers instead of two interstellar ones.
/// The offset only has to stay small enough that `f32` resolves it; a light-year is three
/// orders inside that, and at the rates a crossing runs it means a rebuild every few seconds.
pub const REBAKE_LY: f64 = 1.0;

/// How one class of star is drawn.
///
/// There are two, and they obey different laws, which is the arrangement Exotic Matters
/// arrived at and this follows. The background is a dome: a catalogue star is at infinity, its
/// drawn size says how bright it is and nothing else, and it does not change as the ship moves.
/// A local star is an object: it has a distance, its size is the angle it actually subtends,
/// and approaching it changes both.
///
/// Collapsing the two into one law is what makes a sky of small dust or a sky of balloons. The
/// size that makes arrival at a star look like arrival is the size that makes the background
/// unreadable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointStyle {
    /// Drawn radius in pixels, faintest to brightest. Pixels rather than an angle because the
    /// eye reads a star chart in pixels: a minimum under one leaves the field looking like dust.
    pub min_px: f32,
    pub max_px: f32,
    /// Extra radius per stop above the window, in units of `min_px`.
    pub glow_radius_gain: f32,
    /// HDR value per stop above the window. What bloom turns into glare.
    pub overflow_gain: f32,
    /// Output gain for a source inside the window.
    pub brightness: f32,
    /// How much of the output the glare carries, against the source itself.
    pub halo_gain: f32,
    /// Exponent of the glare's power-law falloff from the source.
    pub halo_falloff: f32,
    /// How much angular structure the glare carries. Zero leaves it a smooth halo.
    pub corona_strength: f32,
    /// Filaments per radian of sky. Halving it halves their number and doubles their width.
    pub corona_frequency: f32,
    /// Shortest streamer, and how much longer the longest is, as fractions of the quad.
    pub corona_reach_min: f32,
    pub corona_reach_span: f32,
    /// Width of the fade at a streamer's tip.
    pub corona_fade: f32,
    /// Brightness between the streamers, and how much they add on top.
    pub corona_floor: f32,
    pub corona_gain: f32,
    /// How far the corona reaches, in **stellar radii** — a world size, not a screen one.
    pub corona_radii: f32,
}

/// Every knob, with the range a slider should offer and whether it is a corona setting.
///
/// A table rather than a hand-written panel: a knob that exists and has no slider is a knob
/// nobody finds, and the two drift apart the moment one is added.
pub const KNOBS: [(&str, fn(&mut PointStyle) -> &mut f32, f32, f32); 15] = [
    ("min radius px", |s| &mut s.min_px, 0.5, 40.0),
    ("max radius px", |s| &mut s.max_px, 1.0, 120.0),
    ("glare per stop", |s| &mut s.glow_radius_gain, 0.0, 4.0),
    ("brightness", |s| &mut s.brightness, 0.0, 6.0),
    ("overflow per stop", |s| &mut s.overflow_gain, 0.0, 6.0),
    ("halo gain", |s| &mut s.halo_gain, 0.0, 2.0),
    // Down to almost nothing: a falloff under 1 is a broad flat glow rather than a tight one,
    // and the value that looked right was below the range this slider first offered.
    ("halo falloff", |s| &mut s.halo_falloff, 0.05, 4.0),
    ("corona strength", |s| &mut s.corona_strength, 0.0, 1.0),
    ("corona frequency", |s| &mut s.corona_frequency, 1.0, 60.0),
    ("reach shortest", |s| &mut s.corona_reach_min, 0.0, 0.9),
    ("reach spread", |s| &mut s.corona_reach_span, 0.0, 0.9),
    ("tip fade", |s| &mut s.corona_fade, 0.02, 0.8),
    ("corona floor", |s| &mut s.corona_floor, 0.0, 1.5),
    ("corona contrast", |s| &mut s.corona_gain, 0.0, 4.0),
    ("corona radii", |s| &mut s.corona_radii, 1.0, 40.0),
];

/// The background. Small, tight, and it must stay readable as a field of thousands.
pub const DISTANT: PointStyle = PointStyle {
    min_px: 1.0,
    max_px: 3.0,
    glow_radius_gain: 0.12,
    overflow_gain: 0.25,
    brightness: 1.2,
    halo_gain: 0.25,
    // Nothing three pixels across has visible structure, and noise at that size is a shimmer.
    halo_falloff: 1.9,
    corona_strength: 0.0,
    corona_frequency: 11.0,
    corona_reach_min: 0.2,
    corona_reach_span: 0.5,
    corona_fade: 0.28,
    corona_floor: 0.22,
    corona_gain: 1.45,
    corona_radii: em_render::relativistic_starfield_material::DEFAULT_CORONA_RADII,
};

/// Lit bodies: planets, moons, anything reflecting.
///
/// Very nearly the background's style, and that is the point. A planet looks like a star —
/// that is why they were called wandering ones — and what distinguishes it is that it moves,
/// not that it is bigger. Drawn any larger the quad stops being a point source and starts
/// being a visible disc with a square behind it.
pub const BODIES: PointStyle = PointStyle {
    min_px: 1.0,
    max_px: 3.6,
    glow_radius_gain: 0.12,
    overflow_gain: 0.25,
    brightness: 1.2,
    halo_gain: 0.25,
    halo_falloff: 1.9,
    corona_strength: 0.0,
    corona_frequency: 11.0,
    corona_reach_min: 0.2,
    corona_reach_span: 0.37,
    corona_fade: 0.28,
    corona_floor: 0.22,
    corona_gain: 1.45,
    corona_radii: em_render::relativistic_starfield_material::DEFAULT_CORONA_RADII,
};

/// A star whose system the ship is inside. Allowed to dominate the screen, because it does.
/// The ranges do not overlap: crossing the shell boundary is a visible step, and a step is
/// better than a star that shrinks as the ship approaches it.
pub const LOCAL: PointStyle = PointStyle {
    min_px: 4.0,
    max_px: 26.0,
    glow_radius_gain: 1.4,
    overflow_gain: 0.1,
    brightness: 0.2,
    halo_gain: 0.15,
    halo_falloff: 1.4,
    corona_strength: 0.95,
    // Streamers per radian of sky. Halving this halves their number and doubles their width,
    // which is the single lever that matters for how a corona reads.
    corona_frequency: 12.0,
    corona_reach_min: 0.20,
    corona_reach_span: 0.37,
    corona_fade: 0.28,
    corona_floor: 0.22,
    corona_gain: 1.45,
    corona_radii: em_render::relativistic_starfield_material::DEFAULT_CORONA_RADII,
};

/// Where the local shell is, re-exported so the drawing code reads the same as the world code.
pub use lc_world::system::LOCAL_SHELL_LY;

/// Radians per pixel, vertically, for a perspective camera.
pub fn radians_per_pixel(fov_y: f32, viewport_height: f32) -> f32 {
    if viewport_height <= 0.0 {
        return 0.0;
    }
    2.0 * (fov_y * 0.5).tan() / viewport_height
}

/// Quad corners, counter-clockwise.
const QUAD: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

/// One of the two passes: its mesh, its material, and which stars are in it.
/// Which of the three passes, and therefore which style and which mesh contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Distant,
    Local,
    Bodies,
}

pub struct Pass {
    pub mesh: Handle<Mesh>,
    pub material: Handle<RelativisticStarfieldMaterial>,
    pub which: Which,
    pub count: usize,
    /// What was last uploaded, so an unchanged frame writes nothing. Reaching for `get_mut`
    /// marks the asset changed whether or not anything differs, and re-uploads the buffer.
    pub sent: RelativisticStarfieldUniform,
}

/// Where the current meshes were baked, and what is drawing them.
#[derive(Resource)]
pub struct Starfield {
    pub origin_ly: DVec3,
    pub distant: Pass,
    pub local: Pass,
    pub bodies: Pass,
}

impl Starfield {
    fn passes(&mut self) -> [&mut Pass; 3] {
        [&mut self.distant, &mut self.local, &mut self.bodies]
    }
}

/// The style a pass draws with, from the interface.
pub fn style_for(ui: &crate::ui::UiState, which: Which) -> PointStyle {
    match which {
        Which::Distant => ui.distant,
        Which::Local => ui.local,
        Which::Bodies => ui.bodies,
    }
}

#[derive(Component)]
pub struct SkyMesh;

/// Split the sky into the background and the system the ship is in.
pub fn partition(session: &Session) -> (Vec<Point>, Vec<Point>) {
    let mut distant = Vec::with_capacity(session.stars.len());
    let mut local = Vec::new();
    for star in &session.stars {
        let target = if session.distance_to(star) < LOCAL_SHELL_LY { &mut local } else { &mut distant };
        target.push(Point::star(star));
    }
    (distant, local)
}

/// Which star's system the ship is inside, if any.
pub fn local_star(session: &Session) -> Option<&CatalogueStar> {
    session.stars.iter().find(|s| session.distance_to(s) < LOCAL_SHELL_LY)
}

/// `log2` of band radiance against temperature: one column per sample, one row per band.
///
/// Generated from `em_spectra::blackbody`, which is the integral the telescope samples. The
/// starfield and the instrument agreeing is not a convention to maintain — there is one
/// Planck integral and this is a tabulation of it.
///
/// Stored as `log2` because the raw value moves by decades between adjacent samples on the
/// Wien side, where a linear blend of the numbers themselves would be wrong by most of the
/// interval. `R32Float` because a 32-bit float texture is not filterable under WebGPU, so the
/// shader interpolates it itself.
pub fn band_lut() -> Image {
    let mut data: Vec<u8> = Vec::with_capacity(LUT_SAMPLES * BANDS * 4);
    for band in Band::ALL {
        for i in 0..LUT_SAMPLES {
            let log_t = LOG_T_MIN + (LOG_T_MAX - LOG_T_MIN) * i as f32 / (LUT_SAMPLES - 1) as f32;
            let radiance = blackbody::band_radiance(band, 2f64.powf(log_t as f64));
            // Far enough below anything representable to read as dark, and finite.
            let value = if radiance > 0.0 { radiance.log2() as f32 } else { -300.0 };
            data.extend_from_slice(&value.to_le_bytes());
        }
    }
    let mut image = Image::new(
        Extent3d { width: LUT_SAMPLES as u32, height: BANDS as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// One thing drawn as a point source: where it is, and what the shader needs to shade it.
///
/// Stars and lit bodies both become these. A planet reflects its star's spectrum, so it is a
/// blackbody at the star's temperature with a smaller radius — see
/// [`crate::system::effective_radius`] — and one shader serves both.
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub position_ly: DVec3,
    pub teff_k: f32,
    /// What the shader treats as the source's radius. For a lit body this is the effective
    /// radius, not the physical one.
    pub radius_m: f32,
    pub seed: f32,
    /// `(temperature K, radiance over the source's disc, grey deficit, unused)`.
    pub warm: [f32; 4],
}

impl Point {
    pub fn star(s: &CatalogueStar) -> Self {
        Self {
            position_ly: s.position_ly,
            teff_k: s.star.teff_k as f32,
            radius_m: s.star.radius_m as f32,
            // Hashed rather than taken raw so two adjacent catalogue ids do not give two stars
            // the same corona.
            seed: (rng::mix(s.seed()) >> 40) as f32 * 1.0e-3,
            warm: warm_params(s),
        }
    }

    /// A body lit by a star of `star_teff_k`.
    pub fn body(d: &crate::system::Drawable, star_teff_k: f64) -> Self {
        // What it re-radiates, against its own drawn disc rather than the star's.
        let thermal_scale = if d.effective_radius_m > 0.0 {
            (d.radius_m / d.effective_radius_m).powi(2)
        } else {
            0.0
        };
        Self {
            position_ly: d.position_ly,
            teff_k: star_teff_k as f32,
            radius_m: d.effective_radius_m as f32,
            seed: 0.0,
            warm: [d.effective_k as f32, thermal_scale as f32, 0.0, 0.0],
        }
    }
}

/// Four vertices per point, positions relative to `origin_ly`, in render axes.
pub fn build_mesh(points: &[Point], origin_ly: DVec3) -> Mesh {
    let n = points.len();
    let mut positions = Vec::with_capacity(n * 4);
    let mut corners = Vec::with_capacity(n * 4);
    let mut params = Vec::with_capacity(n * 4);
    let mut warm = Vec::with_capacity(n * 4);
    let mut indices = Vec::with_capacity(n * 6);

    for point in points {
        // Differenced in f64 and narrowed after, which is the whole point of the bake origin.
        let at = sim_to_render(point.position_ly - origin_ly).as_vec3().to_array();
        let physics = [point.teff_k, point.radius_m, point.seed, 0.0];
        let base = positions.len() as u32;
        for corner in QUAD {
            positions.push(at);
            corners.push(corner);
            params.push(physics);
            warm.push(point.warm);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(ATTRIBUTE_STAR_CORNER, corners);
    mesh.insert_attribute(ATTRIBUTE_STAR_PARAMS, params);
    mesh.insert_attribute(ATTRIBUTE_STAR_WARM, warm);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A star's swarm, as the shader wants it: temperature, radiance against the stellar disc, and
/// the grey deficit it removes. Zeros where there is nothing.
///
/// Drawn from the same generator the telescope reads, so a star that measures as engineered
/// also looks engineered. The swarm is separable from the rest of the system on purpose: this
/// runs for every star in the sky, and generating a full planetary system for each would not.
pub fn warm_params(star: &CatalogueStar) -> [f32; 4] {
    let Some(swarm) = generate::swarm_for(star) else { return [0.0; 4] };
    let r = star.star.radius_m;
    [
        swarm.equilibrium_temperature(&star.star) as f32,
        // The same ratio `reradiated_radiance` applies, and for the same reason: expressed
        // against the stellar disc, the shader's existing geometry term carries it.
        (swarm.radiating_area() / (4.0 * std::f64::consts::PI * r * r)) as f32,
        // Isotropic, so the deficit along any line of sight is the absorbed fraction.
        swarm.absorbed_fraction() as f32,
        0.0,
    ]
}

/// The uniforms that change: where the ship is, how fast, and how it is looking.
pub fn uniforms(
    session: &Session,
    eye_ly: DVec3,
    origin_ly: DVec3,
    lut_scale: f32,
    rad_per_px: f32,
    style: PointStyle,
) -> RelativisticStarfieldUniform {
    let defaults = RelativisticStarfieldUniform::default();
    let radius = |px: f32, fallback: f32| if rad_per_px > 0.0 { px * rad_per_px } else { fallback };
    let mapping = &session.mapping;
    RelativisticStarfieldUniform {
        band_to_display: band_columns(mapping),
        beta: sim_to_render(session.ship.motion.beta).as_vec3().extend(0.0),
        ship_offset_ly: sim_to_render(eye_ly - origin_ly).as_vec3().extend(0.0),
        reference: session.tone.reference,
        point_stops: POINT_STOPS,
        min_radius_rad: radius(style.min_px, defaults.min_radius_rad),
        max_radius_rad: radius(style.max_px, defaults.max_radius_rad),
        glow_radius_gain: style.glow_radius_gain,
        overflow_gain: style.overflow_gain,
        brightness: style.brightness,
        halo_gain: style.halo_gain,
        corona_strength: style.corona_strength,
        corona_frequency: style.corona_frequency,
        halo_falloff: style.halo_falloff,
        corona_reach_min: style.corona_reach_min,
        corona_reach_span: style.corona_reach_span,
        corona_fade: style.corona_fade,
        corona_floor: style.corona_floor,
        corona_gain: style.corona_gain,
        corona_radii: style.corona_radii,
        log_t_min: LOG_T_MIN,
        log_t_scale: lut_scale,
        lut_samples: LUT_SAMPLES as f32,
        ..default()
    }
}

/// The mapping by column: entry `b` is band `b`'s contribution to red, green and blue.
///
/// Unavailable bands are zeroed here rather than in the shader, matching `BandMapping::apply`:
/// an instrument that cannot sense a band contributes nothing through it.
pub fn band_columns(mapping: &BandMapping) -> [Vec4; BANDS] {
    std::array::from_fn(|b| {
        let band = Band::ALL[b];
        if !mapping.available.contains(band) {
            return Vec4::ZERO;
        }
        Vec4::new(mapping.matrix[0][b], mapping.matrix[1][b], mapping.matrix[2][b], 0.0)
    })
}

/// Samples of the table per stop of temperature.
pub fn lut_scale() -> f32 {
    (LUT_SAMPLES - 1) as f32 / (LOG_T_MAX - LOG_T_MIN)
}

/// Radians per pixel for the camera the sky is drawn for.
pub fn camera_scale(camera: &Query<(&Projection, &Camera), With<crate::app::SkyCamera>>) -> f32 {
    let Ok((projection, camera)) = camera.single() else { return 0.0 };
    let Projection::Perspective(perspective) = projection else { return 0.0 };
    let height = camera.logical_viewport_size().map(|s| s.y).unwrap_or(0.0);
    radians_per_pixel(perspective.fov, height)
}

pub fn spawn_sky(
    mut commands: Commands,
    session: Res<crate::app::Game>,
    ui: Res<crate::app::Ui>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RelativisticStarfieldMaterial>>,
    mut images: ResMut<Assets<Image>>,
    camera: Query<(&Projection, &Camera), With<crate::app::SkyCamera>>,
    existing: Query<Entity, With<SkyMesh>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let origin_ly = session.ship.motion.position_ly;
    let rad_per_px = camera_scale(&camera);
    // One table, shared: it is a function of temperature and nothing else.
    let lut = images.add(band_lut());
    let (distant_stars, local_stars) = partition(&session.0);

    let mut pass = |stars: &[Point], which: Which| {
        let style = style_for(&ui.0, which);
        // The ship's own position, not the eye's: this runs on entering the world, before
        // anything has placed one, and the boom is corrected on the very next frame anyway.
        let uniform = uniforms(&session.0, origin_ly, origin_ly, lut_scale(), rad_per_px, style);
        let mesh = meshes.add(build_mesh(stars, origin_ly));
        let material = materials.add(RelativisticStarfieldMaterial {
            uniforms: uniform.clone(),
            band_lut: lut.clone(),
        });
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            // Positions are light-year offsets the shader never uses for placement, so the
            // mesh's bounds say nothing about where it lands on screen.
            NoFrustumCulling,
            SkyMesh,
        ));
        Pass { mesh, material, which, count: stars.len(), sent: uniform }
    };

    let distant = pass(&distant_stars, Which::Distant);
    let local = pass(&local_stars, Which::Local);
    let bodies = pass(&[], Which::Bodies);
    commands.insert_resource(Starfield { origin_ly, distant, local, bodies });
}

/// This frame's view of the local system.
///
/// The system itself lives on the [`Session`], which is what a course is set against; this is
/// only what the renderer made of it, kept so the envelope and surface passes can place rings
/// and spheres without propagating anything a second time.
#[derive(Resource, Default)]
pub struct Bodies {
    pub drawn: Vec<crate::system::Drawable>,
}

/// Load, propagate and re-mesh the local system's bodies.
///
/// The mesh is rebuilt every frame rather than on a threshold, because unlike the stars these
/// move: a body's whole reason to be drawn is that it is somewhere different from last frame.
/// A few hundred bodies is a thousand vertices, which is nothing.
pub fn update_bodies(
    mut session: ResMut<crate::app::Game>,
    eye: Res<crate::hull::Eye>,
    mut bodies: ResMut<Bodies>,
    mut sky: ResMut<Starfield>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    // Coordinate time, not retarded. Inside a system the delay is minutes to hours and moves a
    // planet by far less than a pixel; between systems there is nothing to draw.
    session.0.sync_system();

    let origin = sky.origin_ly;
    // Where the picture is taken from, which is a boom's length behind the ship. A body is
    // solved for and drawn against the same point, so the two cannot disagree.
    let at = eye.at_ly;
    let now = session.0.coordinate_time_s();
    let (drawn, teff) = match session.0.system.as_ref() {
        Some(system) => (system.drawables_at(at, now), system.star_teff_k()),
        None => (Vec::new(), 0.0),
    };
    let points: Vec<Point> = drawn.iter().map(|d| Point::body(d, teff)).collect();
    bodies.drawn = drawn;

    if points.len() == sky.bodies.count && points.is_empty() {
        return;
    }
    if let Some(mut mesh) = meshes.get_mut(&sky.bodies.mesh) {
        *mesh = build_mesh(&points, origin);
    }
    sky.bodies.count = points.len();
}

/// Push this frame's uniforms, and re-bake if the ship has outrun the origin or left a system.
pub fn update_sky(
    session: Res<crate::app::Game>,
    ui: Res<crate::app::Ui>,
    eye: Res<crate::hull::Eye>,
    mut sky: ResMut<Starfield>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RelativisticStarfieldMaterial>>,
    camera: Query<(&Projection, &Camera), With<crate::app::SkyCamera>>,
) {
    let (distant_stars, local_stars) = partition(&session.0);
    // Membership as well as distance: crossing into a system moves a star from one pass to the
    // other, and nothing about the ship's position alone says that happened.
    let moved = session.ship.motion.position_ly.distance(sky.origin_ly) > REBAKE_LY;
    if moved || local_stars.len() != sky.local.count {
        sky.origin_ly = session.ship.motion.position_ly;
        sky.distant.count = distant_stars.len();
        sky.local.count = local_stars.len();
        for (handle, stars) in
            [(sky.distant.mesh.clone(), &distant_stars), (sky.local.mesh.clone(), &local_stars)]
        {
            if let Some(mut mesh) = meshes.get_mut(&handle) {
                *mesh = build_mesh(stars, sky.origin_ly);
            }
        }
    }

    let rad_per_px = camera_scale(&camera);
    let origin = sky.origin_ly;
    for pass in sky.passes() {
        let style = style_for(&ui.0, pass.which);
        let next = uniforms(&session.0, eye.at_ly, origin, lut_scale(), rad_per_px, style);
        if next == pass.sent {
            continue;
        }
        if let Some(mut material) = materials.get_mut(&pass.material) {
            material.uniforms = next.clone();
            pass.sent = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use em_spectra::presets;
    use lc_world::sky::{AuthoredStars, StarProvider};

    use super::*;

    /// The shader's table read, mirrored on the CPU so the tabulation can be checked against
    /// the function it tabulates.
    fn lookup(lut: &Image, band: Band, teff: f64) -> f64 {
        let last = (LUT_SAMPLES - 1) as f32;
        let at = ((teff.log2() as f32 - LOG_T_MIN) * lut_scale()).clamp(0.0, last);
        let (i, j) = (at.floor() as usize, (at.floor() as usize + 1).min(LUT_SAMPLES - 1));
        let row = band.index() * LUT_SAMPLES;
        let texel = |k: usize| {
            let at = (row + k) * 4;
            f32::from_le_bytes(lut.data.as_ref().unwrap()[at..at + 4].try_into().unwrap())
        };
        let frac = at - at.floor();
        let log2 = texel(i) + (texel(j) - texel(i)) * frac;
        2f64.powf(log2 as f64)
    }

    #[test]
    fn the_table_is_the_shape_the_shader_indexes() {
        let lut = band_lut();
        assert_eq!(lut.width(), LUT_SAMPLES as u32);
        assert_eq!(lut.height(), BANDS as u32);
        assert_eq!(lut.data.as_ref().unwrap().len(), LUT_SAMPLES * BANDS * 4);
    }

    /// The claim the whole design rests on: the starfield and the telescope are one
    /// implementation. The shader reads a table; the table has to be the function.
    #[test]
    fn the_table_reproduces_the_planck_integral_the_telescope_samples() {
        let lut = band_lut();
        let mut worst: f64 = 0.0;
        for band in Band::ALL {
            for k in 0..=120 {
                let span = (LOG_T_MAX - LOG_T_MIN) as f64;
                let teff = 2f64.powf(LOG_T_MIN as f64 + span * k as f64 / 120.0);
                let want = blackbody::band_radiance(band, teff);
                if want <= 0.0 || !want.is_finite() {
                    continue;
                }
                let got = lookup(&lut, band, teff);
                worst = worst.max((got - want).abs() / want);
            }
        }
        assert!(worst < 0.005, "table is off the integral by {:.3}%", worst * 100.0);
    }

    /// Why the table holds log2 rather than the radiance: on the Wien side the value moves by
    /// decades between samples, and blending the raw numbers is wrong by most of the interval.
    #[test]
    fn interpolating_the_raw_radiance_would_be_far_worse() {
        let lut = band_lut();
        // A cool star in B: 3000 K is deep on the Wien side of a 445 nm band, which is where
        // the curve is steepest and where any interpolation is hardest.
        let band = Band::B;
        let step = 1.0 / lut_scale() as f64;
        let below = ((3000f64.log2() - LOG_T_MIN as f64) / step).floor();
        let teff = 2f64.powf(LOG_T_MIN as f64 + (below + 0.5) * step);
        let want = blackbody::band_radiance(band, teff);
        assert!(want > 0.0, "the comparison needs a band that is actually emitting");

        let lo = blackbody::band_radiance(band, 2f64.powf(LOG_T_MIN as f64 + below * step));
        let hi = blackbody::band_radiance(band, 2f64.powf(LOG_T_MIN as f64 + (below + 1.0) * step));
        let naive = 0.5 * (lo + hi);

        let ours = (lookup(&lut, band, teff) - want).abs() / want;
        let theirs = (naive - want).abs() / want;
        assert!(ours < theirs, "log2 {ours} should beat linear {theirs}");
    }

    /// The table has to cover the shifted temperature, not the emitted one.
    #[test]
    fn the_table_spans_what_a_relativistic_shift_can_reach() {
        let max_beta = crate::flight::Drive::DEFAULT.max_beta;
        let gamma = (1.0 - max_beta * max_beta).sqrt().recip();
        let toward = gamma * (1.0 + max_beta);
        let coolest = 2000.0 / toward;
        let hottest = 50_000.0 * toward;
        assert!(2f64.powf(LOG_T_MIN as f64) < coolest, "{coolest} K falls off the cold end");
        assert!(2f64.powf(LOG_T_MAX as f64) > hottest, "{hottest} K falls off the hot end");
    }

    /// The columns the shader multiplies must reproduce what `BandMapping::apply` computes,
    /// including the masking: an unavailable band is not dark, it contributes nothing.
    #[test]
    fn the_band_columns_reproduce_the_mapping() {
        for (_, mapping) in presets::all() {
            let radiance = em_spectra::PerBand::new(std::array::from_fn(|i| 1.0 + i as f32));
            let want = mapping.apply(&radiance);
            let columns = band_columns(&mapping);
            let mut got = Vec3::ZERO;
            for b in 0..BANDS {
                got += columns[b].truncate() * radiance[Band::ALL[b]];
            }
            assert!((got - Vec3::from_array(want)).length() < 1e-5, "{got:?} vs {want:?}");
        }
    }

    #[test]
    fn an_unavailable_band_contributes_nothing() {
        let mut mapping = presets::natural();
        mapping.available = em_spectra::BandMask::EMPTY;
        assert_eq!(band_columns(&mapping), [Vec4::ZERO; BANDS]);
    }

    fn sky() -> Session {
        Session::new(&AuthoredStars::sample(), 3)
    }

    #[test]
    fn the_mesh_is_one_quad_per_star() {
        let s = sky();
        let mesh = build_mesh(&points_of(&s), DVec3::ZERO);
        assert_eq!(mesh.count_vertices(), s.stars.len() * 4);
        assert_eq!(mesh.indices().unwrap().len(), s.stars.len() * 6);
    }

    /// The bake origin is the point of the whole arrangement: positions come out small and
    /// the shader differences two small numbers rather than two interstellar ones.
    #[test]
    fn positions_are_relative_to_the_bake_origin() {
        let s = sky();
        let star = &s.stars[0];
        let far = build_mesh(&points_of(&s), DVec3::ZERO);
        let near = build_mesh(&points_of(&s), star.position_ly);
        let first = |m: &Mesh| match m.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            bevy_mesh::VertexAttributeValues::Float32x3(v) => Vec3::from_array(v[0]),
            _ => panic!("positions must be Float32x3"),
        };
        assert!((first(&far).length() as f64 - star.position_ly.length()).abs() < 1e-3);
        assert!(first(&near).length() < 1e-3, "baking at the star puts it at the origin");
    }

    #[test]
    fn the_uniforms_follow_the_ship() {
        let mut s = sky();
        let origin = s.ship.motion.position_ly;
        let eye = |s: &Session| s.ship.motion.position_ly;
        assert_eq!(uniforms(&s, eye(&s), origin, lut_scale(), 0.0, DISTANT).ship_offset_ly, Vec4::ZERO);
        s.fly_to(s.stars[0].id);
        s.advance(8_000.0);
        let u = uniforms(&s, eye(&s), origin, lut_scale(), 0.0, DISTANT);
        assert!(u.ship_offset_ly.truncate().length() > 0.0, "the ship moved and the uniform did not");
        assert!(u.beta.truncate().length() > 0.5, "and it is moving fast");
        assert!(u.beta.truncate().length() < 1.0, "but not at or above c");
    }

    /// A rebake has to leave the sky where it was: the offset absorbs exactly what the
    /// positions gave up.
    #[test]
    fn rebaking_about_a_new_origin_does_not_move_the_sky() {
        let s = sky();
        let star = &s.stars[0];
        let ship = DVec3::new(0.3, -0.2, 0.1);
        let seen = |origin: DVec3| {
            let mesh_pos = sim_to_render(star.position_ly - origin).as_vec3();
            let offset = sim_to_render(ship - origin).as_vec3();
            (mesh_pos - offset).normalize()
        };
        let before = seen(DVec3::ZERO);
        let after = seen(ship);
        assert!((before - after).length() < 1e-6, "{before:?} vs {after:?}");
    }

    /// The renderer and the instrument have to agree about a swarm, or a star measures as
    /// engineered and looks ordinary.
    #[test]
    fn the_warm_attribute_matches_what_the_emission_model_computes() {
        let stars = lc_world::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv");
        let Ok(provider) = stars else { return };
        let swarmed: Vec<_> = provider
            .stars()
            .iter()
            .filter(|s| generate::swarm_for(s).is_some())
            .take(8)
            .collect();
        assert!(!swarmed.is_empty(), "the catalogue should hold some swarms");

        for star in swarmed {
            let [teff, scale, deficit, _] = warm_params(star);
            let swarm = generate::swarm_for(star).unwrap();
            assert!(teff > 0.0 && scale > 0.0, "{teff} K, scale {scale}");
            assert!((0.0..=1.0).contains(&deficit), "a deficit must be a fraction: {deficit}");

            // The shader computes band_radiance(b, teff) * scale; the model computes the same
            // thing through reradiated_radiance. They must be one number.
            let want = swarm.reradiated_radiance(&star.star)[Band::ThermalIr];
            let got = blackbody::band_radiance(Band::ThermalIr, teff as f64) * scale as f64;
            assert!((got / want - 1.0).abs() < 1e-5, "{got} against {want}");
        }
    }

    #[test]
    fn a_star_with_nothing_around_it_carries_no_heat() {
        let s = sky();
        for star in &s.stars {
            if generate::swarm_for(star).is_none() {
                assert_eq!(warm_params(star), [0.0; 4]);
            }
        }
    }

    /// The bug this exists for: a minimum radius written as an angle came out at 0.36 pixels
    /// on a 1280-wide window at 90 degrees, and the whole field rendered as dust.
    #[test]
    fn a_point_source_is_sized_in_pixels_not_in_arcminutes() {
        let fov = std::f32::consts::FRAC_PI_2;
        for height in [480.0, 720.0, 1440.0, 2160.0] {
            let rad_per_px = radians_per_pixel(fov, height);
            // The same star covers the same pixels whatever the window is.
            let drawn = DISTANT.min_px * rad_per_px * height / (2.0 * (fov * 0.5).tan());
            assert!((drawn - DISTANT.min_px).abs() < 1e-3, "{height}px window drew {drawn}");
        }
        assert!(DISTANT.min_px >= 1.0, "anything under a pixel is invisible");
    }

    /// The two passes exist because one law cannot serve both. The size that makes arriving at
    /// a star look like arriving is the size that makes a field of thousands unreadable.
    #[test]
    fn the_background_stays_small_and_a_local_star_is_allowed_to_dominate() {
        assert!(DISTANT.max_px < 4.0, "a background star must not become a ball");
        assert!(LOCAL.min_px > DISTANT.max_px, "the two ranges should not even overlap");
        // Deliberately nothing here about which pass is brighter or glares more. An earlier
        // version asserted the local star carried more overflow, which was a belief about how
        // it would look rather than a requirement, and tuning against a live star overturned
        // it. What has to hold is that the two are separable in size and that the local one is
        // the larger; everything else is taste and belongs on a slider.
        assert_ne!(LOCAL, DISTANT);
    }

    #[test]
    fn a_star_becomes_local_only_inside_its_shell() {
        let mut s = sky();
        let id = s.stars[0].id;
        let (_, local) = partition(&s);
        assert!(local.is_empty(), "four light-years out, nothing is local");

        s.fly_to(id);
        s.advance(40_000.0);
        let (distant, local) = partition(&s);
        assert_eq!(local.len(), 1, "arriving should put exactly the destination in the system");
        let want = s.star(id).unwrap().position_ly;
        assert!(local[0].position_ly.distance(want) < 1e-12, "the wrong star went local");
        assert_eq!(distant.len() + local.len(), s.stars.len(), "no star may be in both or neither");
    }

    #[test]
    fn the_two_passes_are_drawn_with_different_uniforms() {
        let s = sky();
        let rad = radians_per_pixel(std::f32::consts::FRAC_PI_2, 720.0);
        let far = uniforms(&s, DVec3::ZERO, DVec3::ZERO, lut_scale(), rad, DISTANT);
        let near = uniforms(&s, DVec3::ZERO, DVec3::ZERO, lut_scale(), rad, LOCAL);
        assert!(near.max_radius_rad > far.max_radius_rad * 5.0);
        // But they read the same sky: same exposure, same velocity, same table.
        assert_eq!(near.reference, far.reference);
        assert_eq!(near.beta, far.beta);
        assert_eq!(near.log_t_scale, far.log_t_scale);
    }

    #[test]
    fn a_narrower_field_of_view_makes_a_pixel_a_smaller_angle() {
        let wide = radians_per_pixel(std::f32::consts::FRAC_PI_2, 1080.0);
        let narrow = radians_per_pixel(std::f32::consts::FRAC_PI_8, 1080.0);
        assert!(narrow < wide, "zooming in must not grow every star: {narrow} against {wide}");
    }

    #[test]
    fn a_camera_that_is_not_there_falls_back_to_the_defaults() {
        let s = sky();
        let u = uniforms(&s, DVec3::ZERO, DVec3::ZERO, lut_scale(), 0.0, DISTANT);
        let d = RelativisticStarfieldUniform::default();
        assert_eq!(u.min_radius_rad, d.min_radius_rad);
        assert_eq!(u.max_radius_rad, d.max_radius_rad);
        assert!(d.min_radius_rad > 0.0, "and the default is not zero");
    }

    /// Sixty astronomical units from a star is twenty-four stops above four light-years, and
    /// the difference has to survive to the screen or arriving somewhere looks like arriving
    /// nowhere.
    #[test]
    fn arriving_at_a_star_is_a_change_of_many_stops() {
        let mut s = sky();
        let id = s.stars[0].id;
        let flux = |s: &Session| {
            let star = s.star(id).unwrap();
            s.luminance_from(star)
        };
        let far = flux(&s);
        s.fly_to(id);
        s.advance(40_000.0);
        let near = flux(&s);
        let stops = (near / far).log2();
        assert!(stops > 20.0, "arrival should be tens of stops brighter, got {stops}");
    }

    fn points_of(s: &Session) -> Vec<Point> {
        s.stars.iter().map(Point::star).collect()
    }

    fn seeds(mesh: &Mesh) -> Vec<f32> {
        match mesh.attribute(ATTRIBUTE_STAR_PARAMS).unwrap() {
            bevy_mesh::VertexAttributeValues::Float32x4(v) => v.iter().map(|p| p[2]).collect(),
            _ => panic!("params must be Float32x4"),
        }
    }

    /// The corona is noise, and noise that is not deterministic is a star that shimmers
    /// differently for every player. It has to be a function of the star and nothing else --
    /// not of time, not of the camera, not of which pass it landed in.
    #[test]
    fn a_star_gets_the_same_corona_every_time_it_is_baked() {
        let s = sky();
        let a = seeds(&build_mesh(&points_of(&s), DVec3::ZERO));
        let b = seeds(&build_mesh(&points_of(&s), DVec3::new(3.0, -1.0, 2.0)));
        assert_eq!(a, b, "a different bake origin must not change the threads");
        assert!(a.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn two_stars_do_not_share_a_corona() {
        let s = sky();
        let mut per_star: Vec<f32> = seeds(&build_mesh(&points_of(&s), DVec3::ZERO));
        // Four vertices per star carry the same seed; one per star is what must differ.
        per_star.dedup();
        assert_eq!(per_star.len(), s.stars.len(), "adjacent catalogue ids collided: {per_star:?}");
    }

    /// A three-pixel dot has no room for structure, and noise at that size is a shimmer.
    #[test]
    fn only_a_local_star_gets_a_corona() {
        assert_eq!(DISTANT.corona_strength, 0.0);
        assert!(LOCAL.corona_strength > 0.5);
    }

    /// A planet is a point source like any other. Drawn larger than the background's brightest
    /// star it stops reading as a point and starts reading as a disc on a quad.
    #[test]
    fn a_lit_body_is_drawn_no_larger_than_a_bright_star() {
        assert!(BODIES.max_px <= DISTANT.max_px * 1.5, "{} against {}", BODIES.max_px, DISTANT.max_px);
        assert!(BODIES.min_px >= 1.0, "still at least a pixel");
        assert_eq!(BODIES.corona_strength, 0.0);
    }
}
