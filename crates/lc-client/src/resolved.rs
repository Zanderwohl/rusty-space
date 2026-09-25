//! Bodies close enough to be discs rather than points.
//!
//! The point passes draw a body at its *effective* radius, which is a photometric fiction: the
//! radius a blackbody would need to deliver the same flux. That is right for anything
//! unresolved and wrong the moment a body is close enough to have a shape, which is why
//! Saturn's rings had nothing in the middle of them.
//!
//! A resolved body is a sphere instead, with a generated surface and a real terminator. The
//! crossover is angular: past a few pixels the disc is drawn, below it the point is.

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use em_render::atmosphere_material::{AtmosphereMaterial, AtmosphereUniform, TOP_HEIGHTS};
use em_render::body_surface_material::{BANDS, BodySurfaceMaterial, BodySurfaceUniform, GROUNDS};
use em_render::render_space::sim_to_render;
use em_spectra::{Band, BandMapping, PerBand, blackbody, presets};
use glam::DVec3;

use crate::session::{Disc, Scene, Session};
use crate::system::{Drawable, M_PER_LY, UNIT_M};

/// Angular radius, in pixels, past which a body is drawn as a disc.
///
/// Low, because the crossover is where the two agree least: a point is drawn at a minimum size
/// whatever its true angle, so leaving it as a point any longer means it stops growing while
/// its rings keep growing around it.
pub const RESOLVE_PX: f32 = 2.0;

/// Longitude and latitude divisions. Only a handful of bodies are ever resolved, so this can be
/// generous; a visible polygon on a limb is the one artifact that reads as a mistake.
pub const LONGITUDES: u32 = 96;
pub const LATITUDES: u32 = 48;

/// Light on the night side, as a fraction. Not zero: a hard terminator to black turns the
/// unlit half into a bite taken out of the star field.
pub const NIGHT: f32 = 0.012;

/// How far a banded surface's pattern inverts in its own infrared light.
///
/// Jupiter's belts are dark in the optical and *bright* at five microns, and it is the same
/// fact twice: a belt is a gap in the cloud deck, so it reflects less and lets more of the warm
/// interior out. Mean-preserving, so switching bands moves the pattern about rather than
/// changing how much light the body sends.
const INVERSION: f32 = 0.3;

/// `reflected.w` times [`crate::surfaces::RELIEF_SCALE`]. Two because the unit sphere's
/// diameter is one graph sample unit, so this draws the relief at its true slopes.
const BUMP: f32 = 2.0;

/// Which body a resolved sphere stands for.
#[derive(Component)]
pub struct ResolvedBody(pub String);

/// The shell a resolved body's air is drawn on, and that child of the sphere.
#[derive(Component)]
pub struct ResolvedAir(pub Handle<AtmosphereMaterial>, pub Entity);

/// The unit sphere every resolved body shares.
#[derive(Resource, Default)]
pub struct Resolved {
    pub mesh: Option<Handle<Mesh>>,
}

/// Whether a body is close enough to be worth drawing as a sphere.
pub fn is_resolved(body: &Drawable, observer_ly: DVec3, rad_per_px: f32) -> bool {
    if rad_per_px <= 0.0 {
        return false;
    }
    let distance_m = observer_ly.distance(body.position_ly) * M_PER_LY;
    if distance_m <= 0.0 {
        return true;
    }
    (body.radius_m / distance_m) as f32 / rad_per_px > RESOLVE_PX
}

/// The tone map's level for a lit surface: where it sits in the displayed window.
///
/// Computed here rather than in the shader because this is where the tone map and the band
/// mapping live, and a body has to sit in the same exposure as the sky behind it.
///
/// A lit surface's radiance is the star's own, scaled by the albedo and by the solid angle the
/// star covers from there — see [`surface_radiance`].
pub fn surface_level(session: &Session, body: &Drawable, star_radius_m: f64, star_teff_k: f64,
    star_distance_m: f64) -> f32 {
    let radiance = surface_radiance(body, star_radius_m, star_teff_k, star_distance_m);
    // `value`, not `point_brightness`. A surface is an extended source: it either fits the
    // displayed window or it clips, where a point source is spread over a much wider range
    // because its size can carry what its value cannot.
    //
    // `shade_surface`, not `shade`: a surface is metered against a radiance and a point
    // against a flux. [`sample_scene`] is what keeps the two references on the same scene.
    session.tone.shade_surface(&radiance, &session.mapping).value
}

/// How much sky a body covers from `observer_ly`, steradians.
pub fn solid_angle_sr(body: &Drawable, observer_ly: DVec3) -> f32 {
    let distance_m = observer_ly.distance(body.position_ly) * M_PER_LY;
    if distance_m <= 0.0 {
        return 0.0;
    }
    (std::f64::consts::PI * (body.radius_m / distance_m).powi(2)) as f32
}

/// Everything an unresolved body sends the ship: reflected starlight off its effective disc,
/// plus what it re-radiates off its real one. The same two terms the point shader adds.
fn point_flux(body: &Drawable, star_teff_k: f64, observer_ly: DVec3) -> PerBand<f32> {
    let distance_m = observer_ly.distance(body.position_ly) * M_PER_LY;
    if distance_m <= 0.0 {
        return PerBand::splat(0.0);
    }
    let reflected = crate::session::bare(star_teff_k, body.effective_radius_m, distance_m);
    let thermal = crate::session::bare(body.effective_k, body.radius_m, distance_m);
    PerBand::new(std::array::from_fn(|i| reflected[Band::ALL[i]] + thermal[Band::ALL[i]]))
}

/// Bolometric flux from an unresolved body, up to a factor every body shares.
///
/// Both terms of [`point_flux`] with the Planck integral replaced by the Stefan-Boltzmann law
/// it integrates to, which is what makes it cheap: no band integrals at all. Used only to rank.
///
/// Bolometric, where the metering is through a band mapping, so a preset narrow enough to sit
/// on one side of two bodies' spectra could rank them the other way round. It decides only
/// which of two hundred bodies are worth shading, and both would have to be inside the couple
/// of per cent that clips for the difference to reach the screen.
fn point_power(body: &Drawable, star_teff_k: f64, observer_ly: DVec3) -> f64 {
    let distance_m = observer_ly.distance(body.position_ly) * M_PER_LY;
    if distance_m <= 0.0 {
        return 0.0;
    }
    let reflected = star_teff_k.powi(4) * (body.effective_radius_m / distance_m).powi(2);
    let thermal = body.effective_k.powi(4) * (body.radius_m / distance_m).powi(2);
    reflected + thermal
}

/// How many unresolved bodies are metered by their spectrum rather than dropped.
///
/// A point can only move the exposure by crowding into the couple of per cent of the frame
/// that is allowed to clip, which against a six thousand star field is a hundred-odd slots. Two
/// hundred and thirty solar system bodies cost two and a half milliseconds a frame to shade and
/// all but the brightest handful are thirty stops under the cut. Ranking by
/// [`point_power`] first costs nothing and throws away nothing that could have counted.
const METERED_POINTS: usize = 32;

/// How far the bodies' share of the frame may move before the exposure is re-placed, in stops.
///
/// Not every frame: re-placing costs a pass over every star in the catalog. A quarter of a
/// stop is below what anyone can see step, and a body's surface radiance does not depend on how
/// far away the ship is at all — only its size on screen does — so an approach crosses this
/// perhaps a few dozen times rather than continuously.
const REMETER_STOPS: f32 = 0.25;

/// Tell the session what the renderer is about to draw, and re-expose when that has moved.
pub fn sample_scene(
    ui: Res<crate::app::Ui>,
    mut game: ResMut<crate::app::Game>,
    bodies: Res<crate::starfield::Bodies>,
    eye: Res<crate::hull::Eye>,
    uplink: Res<crate::uplink::Uplink>,
    own_form: Res<crate::parts::OwnForm>,
    camera: Query<(&Projection, &Camera), With<crate::app::SkyCamera>>,
    mut last: Local<Option<(usize, f32)>>,
) {
    let rad_per_px = crate::starfield::camera_scale(&camera);
    let observer = eye.at_ly;
    let mut scene = Scene { point_sr: rad_per_px * rad_per_px, ..default() };
    // The three numbers rather than the system: the borrow has to end before the scene is
    // written back, and copying two hundred and thirty bodies a frame to avoid that would cost
    // more than the metering it feeds.
    let now = game.0.coordinate_time_s();
    let star = game.0.system.as_ref().and_then(|s| {
        Some((s.star_position_at(now)?, s.star_radius_m(), s.star_teff_k()))
    });
    if let Some((star_ly, star_radius, star_teff)) = star {
        let mut unresolved: Vec<(f64, &Drawable)> = Vec::new();
        for body in &bodies.drawn {
            if is_resolved(body, observer, rad_per_px) {
                let star_distance = star_ly.distance(body.position_ly) * M_PER_LY;
                scene.discs.push(Disc {
                    radiance: surface_radiance(body, star_radius, star_teff, star_distance),
                    solid_angle_sr: solid_angle_sr(body, observer),
                });
            } else {
                unresolved.push((point_power(body, star_teff, observer), body));
            }
        }
        if unresolved.len() > METERED_POINTS {
            unresolved.select_nth_unstable_by(METERED_POINTS, |a, b| b.0.total_cmp(&a.0));
            unresolved.truncate(METERED_POINTS);
        }
        scene.points =
            unresolved.iter().map(|(_, b)| point_flux(b, star_teff, observer)).collect();
    }

    // Hulls, the player's own included. A ship filling half the frame is the brightest thing
    // in it, and an exposure metered without it puts the picture's subject off the top of the
    // window — which is a white blob where the ship is.
    //
    // Asked once: between the stars there is no system to light a hull and the answer is a
    // search over the whole catalog.
    let hull_star = crate::hull::lighting(&game.0);
    // The length the boom was counted in, so the disc and the standoff agree.
    let own_length_m = own_form.length_m().unwrap_or(game.ship.length_m);
    let hulls = std::iter::once((own_length_m, eye.boom_m, observer))
        .chain(uplink.contacts.iter().map(|c| {
            (c.length_m, c.position_ly.distance(observer) * M_PER_LY, c.position_ly)
        }));
    for (length_m, distance_m, at_ly) in hulls {
        scene.discs.push(Disc {
            radiance: crate::hull::radiance_at(hull_star, at_ly),
            solid_angle_sr: crate::hull::solid_angle_sr(length_m, distance_m),
        });
    }

    // The summary is the power the bodies contribute, which moves with both their brightness
    // and their size. The disc count is in it separately so that a body crossing the resolution
    // threshold always re-meters: it changes which term it is counted under.
    let power: f32 = scene.discs.iter().map(|d| luma(&d.radiance) * d.solid_angle_sr).sum::<f32>()
        + scene.points.iter().map(luma).sum::<f32>();
    let now = (scene.discs.len(), if power > 0.0 { power.log2() } else { f32::NEG_INFINITY });
    let moved = match *last {
        Some((discs, before)) => discs != now.0 || (now.1 - before).abs() > REMETER_STOPS,
        None => true,
    };
    game.0.scene = scene;
    if moved {
        *last = Some(now);
        crate::action::refresh_exposure(&ui.0, &mut game.0);
    }
}

/// Unweighted band sum, only ever compared against itself to decide whether to re-meter.
fn luma(radiance: &PerBand<f32>) -> f32 {
    Band::ALL.iter().map(|b| radiance[*b]).sum()
}

/// What a lit surface actually emits, per band: `p (R_star / d)^2 B(T_star)`.
///
/// Separate from the tone mapping because this is the physics and that is the display. The two
/// are far apart in dynamic range: a surface at one astronomical unit and the same surface at
/// thirty are ten stops apart, and the displayed window is two and a half.
pub fn surface_radiance(
    body: &Drawable,
    star_radius_m: f64,
    star_teff_k: f64,
    star_distance_m: f64,
) -> PerBand<f32> {
    let reflected = reflected_radiance(body, star_radius_m, star_teff_k, star_distance_m);
    let emitted = match body.climate {
        // A world with its own temperatures is metered on its day side, which is what fills
        // the picture: at its mean the day side sat three stops over at ten microns, one flat
        // clipped disc.
        Some(c) => blackbody_at(body.effective_k * day_side(c.air.evens)),
        None => emitted_radiance(body),
    };
    PerBand::new(std::array::from_fn(|i| reflected[Band::ALL[i]] + emitted[Band::ALL[i]]))
}

/// The day hemisphere's mean temperature over the body's mean: the average of `(4 cos z)^(1/4)`
/// over it, 1.13 where nothing carries heat round and one where the air evens everything.
fn day_side(evens: f32) -> f64 {
    1.0 + 0.13 * (1.0 - f64::from(evens))
}

fn blackbody_at(k: f64) -> PerBand<f32> {
    if k <= 0.0 {
        return PerBand::splat(0.0);
    }
    PerBand::new(std::array::from_fn(|i| blackbody::band_radiance(Band::ALL[i], k) as f32))
}

/// The starlight half: `p (R_star / d)^2 B(T_star)`, which has the star's spectrum.
pub fn reflected_radiance(
    body: &Drawable,
    star_radius_m: f64,
    star_teff_k: f64,
    star_distance_m: f64,
) -> PerBand<f32> {
    lit_radiance(mean_albedo(body), star_radius_m, star_teff_k, star_distance_m)
}

/// A giant's comes from its chemistry: a cloudless one is a tenth of a water-cloud one's.
fn mean_albedo(body: &Drawable) -> f64 {
    if body.giant.is_some() { body.world.gray_albedo() } else { body.surface.albedo() }
}

/// What the shader multiplies its texel by. A giant's cubemap is its albedo.
fn shading_albedo(body: &Drawable, drawn: crate::surfaces::Drawn) -> f64 {
    if drawn.layers { 1.0 } else { body.surface.albedo() }
}

/// The same law with the albedo given rather than looked up, for anything lit that is not a
/// world. A hull is one; keeping it here is what stops a ship and the planet beside it being
/// shaded by two different formulas.
///
/// The reflected half only. A hull has no [`Drawable::effective_k`] and so no thermal term —
/// which is wrong in the far infrared, where a ship is warm and would show it, and is a gap in
/// the craft model rather than in this.
pub fn lit_radiance(
    albedo: f64,
    star_radius_m: f64,
    star_teff_k: f64,
    star_distance_m: f64,
) -> PerBand<f32> {
    if star_distance_m <= 0.0 {
        return PerBand::splat(0.0);
    }
    let scale = albedo * (star_radius_m / star_distance_m).powi(2);
    PerBand::new(std::array::from_fn(|i| {
        (blackbody::band_radiance(Band::ALL[i], star_teff_k) * scale) as f32
    }))
}

/// The half a body emits itself: a blackbody at whatever it radiates at.
///
/// No geometry in it at all. A surface at `T` has radiance `B(T)` whichever way it is turned and
/// however far away it is, which is why this is the term that does not care about the star — and
/// why a giant's night side is as bright at ten microns as its day side.
///
/// `effective_k`, not `equilibrium_k`: a giant makes heat of its own. Jupiter radiates 1.67
/// times what it takes from the Sun, and at ten microns that is the difference between a body
/// you can see and one you cannot.
pub fn emitted_radiance(body: &Drawable) -> PerBand<f32> {
    blackbody_at(body.effective_k)
}

/// The air's own temperature where it radiates to space, over the body's mean: colder than the
/// ground, which is what makes it a screen at ten microns rather than a mirror.
const AIR_K: f64 = 0.85;

/// A climate's air as scatter.wgsl packs it, per display channel: gas depths and scale height;
/// haze depths and ten-micron depth; haze albedo; and the air's own glow. Zero without air.
///
/// A channel's depth is its bands' depths averaged by the starlight the mapping puts on it
/// from each, so a channel carrying K sees through Rayleigh and one carrying ten microns
/// scatters nothing. A channel with no starlight on it has no air to scatter.
fn air_of(body: &Drawable, mapping: &BandMapping, star: &PerBand<f32>) -> [Vec4; 4] {
    let Some(c) = &body.climate else { return [Vec4::ZERO; 4] };
    let a = c.air;
    let alone = |b: usize, value: f32| {
        Vec3::from_array(mapping.apply(&PerBand::new(std::array::from_fn(|i| if i == b { value } else { 0.0 }))))
    };
    let (mut weight, mut gas, mut haze, mut scattered) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
    for (b, band) in Band::ALL.into_iter().enumerate() {
        let w = alone(b, star[band]);
        let (g, h, albedo) = a.in_band(band);
        weight += w;
        gas += w * g;
        haze += w * h;
        scattered += w * h * albedo;
    }
    let per = |sum: Vec3, over: Vec3| Vec3::select(over.cmpgt(Vec3::ZERO), sum / over, Vec3::ZERO);
    let haze = per(haze, weight);
    let albedo = per(scattered, haze * weight);
    let glow = alone(Band::ThermalIr.index(), blackbody::band_radiance(Band::ThermalIr, body.effective_k * AIR_K) as f32);
    [per(gas, weight).extend(a.height), haze.extend(a.infrared), albedo.extend(0.0), glow.extend(0.0)]
}

/// What the surface reflects and what it emits, each as linear display light.
///
/// Through the band mapping but *not* through the tone map: the two mix differently across the
/// disc, so the curve has to be evaluated per fragment. Tone-mapping them separately and adding
/// the results put Jupiter's day side at twice its night side at ten microns, where the true
/// ratio is 1.14 — the reflected half adds an eighth to a face that is already glowing.
pub fn surface_shading(
    session: &Session,
    body: &Drawable,
    albedo: f64,
    star_radius_m: f64,
    star_teff_k: f64,
    star_distance_m: f64,
) -> (glam::Vec3, glam::Vec3) {
    let reflected = lit_radiance(albedo, star_radius_m, star_teff_k, star_distance_m);
    let emitted = emitted_radiance(body);
    let through = |r| glam::Vec3::from_array(session.mapping.apply(&r));
    (through(reflected), through(emitted))
}

/// Each ground's albedo through `mapping`, in [`BodySurfaceUniform::ground`]'s order.
fn grounds(mapping: &BandMapping, star: &PerBand<f32>, rust: f32) -> [Vec4; GROUNDS] {
    use lc_world::ground::{Ground, rock};
    [
        Ground::Water.reflectance(),
        Ground::Ice.reflectance(),
        Ground::Growth.reflectance(),
        Ground::Sand.reflectance(),
        rock(rust),
        Ground::Cloud.reflectance(),
    ]
    .map(|r| albedo_through(mapping, star, &r))
}

/// The same for a giant's layers; the slots past them are weighted zero.
fn giant_layers(mapping: &BandMapping, star: &PerBand<f32>, giant: &lc_world::giant::Giant) -> [Vec4; GROUNDS] {
    std::array::from_fn(|k| giant.layers.get(k).map_or(Vec4::ONE, |r| albedo_through(mapping, star, r)))
}

fn albedo_through(mapping: &BandMapping, star: &PerBand<f32>, r: &[f32; em_spectra::BANDS]) -> Vec4 {
    let white = mapping.apply(star);
    let lit = mapping.apply(&PerBand::new(std::array::from_fn(|i| r[i] * star[Band::ALL[i]])));
    let albedo: [f32; 3] = std::array::from_fn(|c| if white[c] > 0.0 { lit[c] / white[c] } else { 0.0 });
    Vec3::from_array(albedo).extend(1.0)
}

/// What a surface that knows its grounds is drawn with, in every band.
#[derive(Clone, Copy)]
struct Grounds {
    now: [Vec4; GROUNDS],
    natural: [Vec4; GROUNDS],
    bands: [Vec4; BANDS],
    thermal: Vec4,
    emissivity: [Vec4; 2 * GROUNDS],
}

impl Grounds {
    fn of(mapping: &BandMapping, star: &PerBand<f32>, climate: &lc_world::climate::Climate, mean_k: f64) -> Self {
        use lc_world::ground::{Ground, rock_emissivity, rock_inertia};
        let rust = climate.rust;
        // Each band alone through the mapping, so the shader can weight them one by one.
        let bands = std::array::from_fn(|b| {
            let band = Band::ALL[b];
            let alone = PerBand::new(std::array::from_fn(|i| {
                if i == b { blackbody::band_radiance(band, mean_k) as f32 } else { 0.0 }
            }));
            Vec3::from_array(mapping.apply(&alone)).extend((band.center_m() * 1.0e6) as f32)
        });
        let emissivities = [
            (Ground::Water.emissivity(), Ground::Water.inertia()),
            (Ground::Ice.emissivity(), Ground::Ice.inertia()),
            (Ground::Growth.emissivity(), Ground::Growth.inertia()),
            (Ground::Sand.emissivity(), Ground::Sand.inertia()),
            (rock_emissivity(rust), rock_inertia(rust)),
            (Ground::Cloud.emissivity(), Ground::Cloud.inertia()),
        ];
        let mut emissivity = [Vec4::ZERO; 2 * GROUNDS];
        for (k, (e, inertia)) in emissivities.into_iter().enumerate() {
            emissivity[2 * k] = Vec4::new(e[0], e[1], e[2], e[3]);
            emissivity[2 * k + 1] = Vec4::new(e[4], e[5], e[6], inertia);
        }
        Self {
            now: grounds(mapping, star, rust),
            natural: grounds(&presets::natural(), star, rust),
            bands,
            thermal: Vec4::new(mean_k as f32, climate.air.evens, 0.0, 1.0),
            emissivity,
        }
    }

    /// No temperatures of its own: a giant glows as one blackbody whose belts invert.
    fn giant(mapping: &BandMapping, star: &PerBand<f32>, giant: &lc_world::giant::Giant) -> Self {
        let flat = BodySurfaceUniform::default();
        Self {
            now: giant_layers(mapping, star, giant),
            natural: giant_layers(&presets::natural(), star, giant),
            bands: flat.bands,
            thermal: Vec4::ZERO,
            emissivity: flat.emissivity,
        }
    }
}

fn uniforms(
    body: &Drawable,
    star_ly: DVec3,
    tone: &crate::tonemap::ToneMap,
    reflected: glam::Vec3,
    emitted: glam::Vec3,
    drawn: crate::surfaces::Drawn,
    grounds: Option<Grounds>,
    air: [Vec4; 4],
    weather: Option<crate::surfaces::Weather>,
) -> BodySurfaceUniform {
    let as_weight = |on: bool| f32::from(u8::from(on));
    let (color, clouds) = (as_weight(drawn.color), as_weight(drawn.clouds));
    let flat = BodySurfaceUniform::default();
    let [air_gas, air_haze, air_albedo, air_glow] = air;
    let deck = body.climate.map(|c| c.clouds);
    let (dark, light, contrast) = body.surface.palette();
    let to_star = sim_to_render((star_ly - body.position_ly).normalize_or_zero()).as_vec3();
    BodySurfaceUniform {
        dark: Vec4::new(dark[0], dark[1], dark[2], 1.0),
        light: Vec4::new(light[0], light[1], light[2], 1.0),
        to_star: to_star.extend(NIGHT),
        params: Vec4::new(
            color,
            contrast,
            if weather.is_some() { clouds } else { 0.0 },
            // body_surface.wgsl's `MODE_GROUNDS` and `MODE_LAYERS`.
            if drawn.layers { 2.0 } else { as_weight(drawn.grounds) },
        ),
        reflected: reflected.extend(if drawn.relief { BUMP / crate::surfaces::RELIEF_SCALE } else { 0.0 }),
        // `w` is how far the pattern inverts in the body's own light. See [`INVERSION`].
        emitted: emitted.extend(if body.surface.is_banded() { INVERSION } else { 0.0 }),
        exposure: Vec4::new(tone.surface_reference, tone.surface_stops, 0.0, 0.0),
        weather: weather.map_or(Vec4::ZERO, |w| w.weights),
        drift: weather.map_or(Vec4::ZERO, |w| w.drift),
        deck: deck.map_or(Vec4::new(0.0, 1.0, 0.0, 0.0), |d| Vec4::new(d.cover, d.opacity, 0.0, 0.0)),
        deck_tint: deck.map_or(Vec4::ONE, |d| Vec3::from_array(d.tint).extend(1.0)),
        // The ground's own scale, so the air and the ground keep the ratio they have: the
        // surface's albedo is its cubemap times its class's, and air scaled by white starlight
        // came out three times too bright against it.
        starlight: reflected.extend(0.0),
        air_gas,
        air_haze,
        air_albedo,
        air_glow,
        ground: grounds.map_or(flat.ground, |g| g.now),
        ground_natural: grounds.map_or(flat.ground_natural, |g| g.natural),
        bands: grounds.map_or(flat.bands, |g| g.bands),
        thermal: grounds.map_or(flat.thermal, |g| g.thermal),
        emissivity: grounds.map_or(flat.emissivity, |g| g.emissivity),
    }
}

fn air_uniforms(surface: &BodySurfaceUniform) -> AtmosphereUniform {
    AtmosphereUniform {
        to_star: surface.to_star,
        starlight: surface.starlight,
        exposure: surface.exposure,
        gas: surface.air_gas,
        haze: surface.air_haze,
        albedo: surface.air_albedo,
        glow: surface.air_glow,
    }
}

/// Where a unit sphere goes to be `body`, seen from `eye_ly`.
fn placement(body: &Drawable, eye_ly: DVec3) -> Transform {
    Transform {
        translation: sim_to_render((body.position_ly - eye_ly) * M_PER_LY / UNIT_M).as_vec3(),
        rotation: Quat::from_rotation_arc(Vec3::Y, sim_to_render(body.pole).as_vec3().normalize()),
        scale: Vec3::splat((body.radius_m / UNIT_M) as f32),
    }
}

/// Where a lit surface sits in its window when a photograph is exposed for it, as a fraction.
/// Under the top, so a bright cloud deck keeps its structure.
const SUBJECT_VALUE: f32 = 0.9;

/// The surface reference that puts `radiance`, full on, at [`SUBJECT_VALUE`] of the window.
pub fn metered_for(tone: &crate::tonemap::ToneMap, radiance: &PerBand<f32>, mapping: &BandMapping)
    -> f32 {
    let shaded = tone.shade_surface(radiance, mapping);
    if !shaded.stops.is_finite() || tone.surface_stops <= 0.0 {
        return tone.surface_reference;
    }
    tone.surface_reference * (shaded.stops + (1.0 - SUBJECT_VALUE) * tone.surface_stops).exp2()
}

/// The layer a body's sphere is drawn on, if it has one.
fn sphere_layer(body: &Drawable, eye_ly: DVec3, rad_per_px: f32, shot: &crate::beauty::ShotBody)
    -> Option<usize> {
    if is_resolved(body, eye_ly, rad_per_px) {
        return Some(0);
    }
    (shot.name.as_deref() == Some(body.name.as_str())
        && is_resolved(body, eye_ly, shot.rad_per_px))
        .then_some(crate::beauty::SHOT_LAYER)
}

/// Keep a sphere for every body close enough to be one.
///
/// A sphere lives exactly as long as its body stays resolved, and is placed on the frame it is
/// spawned. Respawning every sphere whenever the set changed, and placing them a frame later,
/// left Earth undrawn for a frame each time its moon crossed the threshold — a black flash.
pub fn update_resolved(
    mut commands: Commands,
    session: Res<crate::app::Game>,
    bodies: Res<crate::starfield::Bodies>,
    eye: Res<crate::hull::Eye>,
    mut resolved: ResMut<Resolved>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodySurfaceMaterial>>,
    mut airs: ResMut<Assets<AtmosphereMaterial>>,
    mut surfaces: ResMut<crate::surfaces::Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut bakes: ResMut<crate::procedural::Bakes>,
    shot: Res<crate::beauty::ShotBody>,
    camera: Query<(&Projection, &Camera), With<crate::app::SkyCamera>>,
    mut placed: Query<(
        Entity,
        &mut Transform,
        &mut RenderLayers,
        &MeshMaterial3d<BodySurfaceMaterial>,
        &ResolvedBody,
        Option<&ResolvedAir>,
    )>,
) {
    let rad_per_px = crate::starfield::camera_scale(&camera);
    let Some(system) = session.0.system.as_ref() else {
        for (entity, ..) in &placed {
            commands.entity(entity).despawn();
        }
        return;
    };
    let star_ly = system.star_position_ly();
    let (star_radius, star_teff) = (system.star_radius_m(), system.star_teff_k());
    let now_s = session.0.coordinate_time_s();
    // A sphere only the telescope draws is exposed for itself. The surface's window is
    // logarithmic, so no later exposure recovers a disc it clipped.
    let mut shade = |body: &Drawable, surfaces: &mut crate::surfaces::Surfaces, own: bool| {
        let star_distance = star_ly.distance(body.position_ly) * M_PER_LY;
        let mut tone = session.tone;
        if own {
            let radiance = surface_radiance(body, star_radius, star_teff, star_distance);
            tone.surface_reference = metered_for(&tone, &radiance, &session.0.mapping);
        }
        let drawn = surfaces.drawn(&body.name);
        let (reflected, emitted) = surface_shading(
            &session.0,
            body,
            shading_albedo(body, drawn),
            star_radius,
            star_teff,
            star_distance,
        );
        let star = lit_radiance(1.0, star_radius, star_teff, star_distance);
        let mapping = &session.0.mapping;
        let ground = match (body.climate, body.giant) {
            (Some(c), _) if drawn.grounds => Some(Grounds::of(mapping, &star, &c, body.effective_k)),
            (_, Some(g)) if drawn.layers => Some(Grounds::giant(mapping, &star, &g)),
            _ => None,
        };
        let air = air_of(body, &session.0.mapping, &star);
        let weather = surfaces.weather(&body.name, now_s, body.radius_m, &mut bakes);
        uniforms(body, star_ly, &tone, reflected, emitted, drawn, ground, air, weather)
    };

    let want: Vec<(&Drawable, RenderLayers)> = bodies
        .drawn
        .iter()
        .filter_map(|d| Some((d, RenderLayers::layer(sphere_layer(d, eye.at_ly, rad_per_px, &shot)?))))
        .collect();
    // Linear searches: only a handful of bodies are ever resolved at once.
    let mut kept: Vec<&str> = Vec::with_capacity(want.len());
    for (entity, mut transform, mut layers, material, marker, air) in placed.iter_mut() {
        let Some((body, layer)) = want.iter().find(|(d, _)| d.name == marker.0) else {
            commands.entity(entity).despawn();
            continue;
        };
        kept.push(&body.name);
        *transform = placement(body, eye.at_ly);
        if *layers != *layer {
            *layers = layer.clone();
            if let Some(air) = air {
                commands.entity(air.1).insert(layer.clone());
            }
        }
        if let Some(mut asset) = materials.get_mut(&material.0) {
            let next = shade(body, &mut surfaces, *layer != RenderLayers::layer(0));
            if let Some(mut shell) = air.and_then(|a| airs.get_mut(&a.0)) {
                let next = air_uniforms(&next);
                if shell.uniforms != next {
                    shell.uniforms = next;
                }
            }
            if asset.uniforms != next {
                asset.uniforms = next;
            }
        }
    }

    for (body, layer) in want.iter().filter(|(d, _)| !kept.contains(&d.name.as_str())) {
        let mesh = resolved
            .mesh
            .get_or_insert_with(|| meshes.add(Sphere::new(1.0).mesh().uv(LONGITUDES, LATITUDES)))
            .clone();
        let own = surfaces.images(&body.name, body.surface, body.climate, body.giant, body.airless, &mut images);
        let uniforms = shade(body, &mut surfaces, *layer != RenderLayers::layer(0));
        let air = air_uniforms(&uniforms);
        let sphere = commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(materials.add(own.material(uniforms))),
                placement(body, eye.at_ly),
                NoFrustumCulling,
                layer.clone(),
                ResolvedBody(body.name.clone()),
            ))
            .id();
        if let Some(climate) = body.climate {
            let shell = airs.add(AtmosphereMaterial { uniforms: air });
            let child = commands
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(shell.clone()),
                    Transform::from_scale(Vec3::splat(1.0 + TOP_HEIGHTS * climate.air.height)),
                    NoFrustumCulling,
                    layer.clone(),
                    ChildOf(sphere),
                ))
                .id();
            commands.entity(sphere).insert(ResolvedAir(shell, child));
        }
    }
}

#[cfg(test)]
mod tests {
    use lc_world::sky::AuthoredStars;
    use lc_world::surface::Surface;

    use crate::session::NOMINAL_POINT_SR;

    use super::*;

    const AU: f64 = 1.495_978_707e11;

    fn body(radius_m: f64, at: DVec3) -> Drawable {
        Drawable {
            mass_kg: 0.0,
            name: "test".into(),
            kind: lc_world::navigation::Kind::Planet,
            rings: None,
            surface: Surface::Rock,
            world: lc_world::worlds::of("test", Surface::Rock, &[], None),
            climate: None,
            giant: None,
            airless: None,
            pole: DVec3::Z,
            spin_s: None,
            position_ly: at,
            radius_m,
            effective_radius_m: 1.0,
            equilibrium_k: 250.0,
            effective_k: Surface::Rock.effective_temperature(250.0),
        }
    }

    /// With I on the red channel a forest is bright and the sea black, which the color map
    /// painted in the natural mapping cannot say; in the natural mapping nothing moves.
    #[test]
    fn each_band_sees_its_own_ground() {
        let star = lit_radiance(1.0, 6.957e8, 5772.0, AU);
        let natural = grounds(&presets::natural(), &star, 0.3);
        let infrared = grounds(&BandMapping::direct(Band::I, Band::V, Band::B), &star, 0.3);
        let [water, _, growth, ..] = [0, 1, 2].map(|k| infrared[k].x / natural[k].x);
        assert!(growth > 5.0, "the red edge: {growth}");
        assert!(water < 0.7, "water darkens past the eye: {water}");
        // Blue and green carry the same bands in both, so they do not move.
        assert!((infrared[2].y - natural[2].y).abs() < 1e-6);
        // K on red: snow goes dark.
        let k = grounds(&BandMapping::direct(Band::K, Band::V, Band::B), &star, 0.3);
        assert!(k[1].x < natural[1].x / 4.0, "snow in K: {} against {}", k[1].x, natural[1].x);
    }

    /// At the mean temperature with unit emissivity the shader's per-band table sums to exactly
    /// what the host would have sent as `emitted`, in any mapping.
    #[test]
    fn the_band_table_sums_to_the_blackbody() {
        let climate = lc_world::climate::of("Earth", lc_world::worlds::for_body("Earth").unwrap(), 278.0, 5772.0, &[]).unwrap();
        let star = lit_radiance(1.0, 6.957e8, 5772.0, AU);
        for (_, mapping) in presets::all() {
            let g = Grounds::of(&mapping, &star, &climate, 255.0);
            let sum: Vec3 = g.bands.iter().map(|b| b.truncate()).sum();
            let blackbody = PerBand::new(std::array::from_fn(|i| blackbody::band_radiance(Band::ALL[i], 255.0) as f32));
            let want = Vec3::from_array(mapping.apply(&blackbody));
            assert!((sum - want).abs().max_element() <= want.max_element() * 1e-5, "{sum} against {want}");
        }
    }

    /// The natural mapping keeps the air it had; K on the red channel makes it clear there, and
    /// ten microns on red scatters nothing and glows instead.
    #[test]
    fn each_channel_scatters_what_its_bands_would() {
        let mut earth = body(6.371e6, DVec3::X * 1.0e-9);
        earth.effective_k = 255.0;
        earth.climate = lc_world::climate::of("Earth", lc_world::worlds::for_body("Earth").unwrap(), 278.0, 5772.0, &[]);
        let star = lit_radiance(1.0, 6.957e8, 5772.0, AU);
        let [gas, ..] = air_of(&earth, &presets::natural(), &star);
        assert!(gas.z > 2.0 * gas.y && gas.y > 1.5 * gas.x, "blue sky: {gas}");
        let [gas, ..] = air_of(&earth, &BandMapping::direct(Band::K, Band::V, Band::B), &star);
        assert!(gas.x < gas.y / 50.0, "K sees through Rayleigh: {gas}");
        let [gas, haze, _, glow] = air_of(&earth, &presets::thermal(), &star);
        assert_eq!((gas.x, haze.x), (0.0, 0.0), "nothing scatters at ten microns");
        assert!(glow.x > 0.0 && glow.y == 0.0, "the air glows only where ten microns is: {glow}");
    }

    /// A Jupiter-like body at Jupiter's distance, so the numbers mean something.
    fn giant() -> Drawable {
        let mut b = body(6.99e7, DVec3::X * 1.0e-9);
        b.surface = Surface::GasGiant;
        // The gray balance at 5.2 AU, which is what `equilibrium_temperature` would give.
        b.equilibrium_k = 122.0;
        b.effective_k = Surface::GasGiant.effective_temperature(122.0);
        b
    }

    /// The whole point: a giant is a reflector in the optical and a source in the infrared.
    ///
    /// Jupiter radiates 1.67 times what it takes from the Sun, and at ten microns its own light
    /// is orders above the sunlight it bounces. In V the reverse holds by a far wider margin,
    /// which is why adding this changes nothing about a planet seen in natural light.
    #[test]
    fn a_giant_is_a_source_in_the_infrared_and_a_mirror_in_the_optical() {
        let jupiter = giant();
        let sun_distance = 5.2044 * AU;
        let reflected = reflected_radiance(&jupiter, 6.957e8, 5772.0, sun_distance);
        let emitted = emitted_radiance(&jupiter);

        // Seven or so, not the hundreds an intuition about "thermal infrared" suggests: ten
        // microns is on a 125 K body's Wien side, and the Sun still has a Rayleigh-Jeans tail
        // there. It is the band where the two are closest to comparable, and the giant wins.
        assert!(
            emitted[Band::ThermalIr] > reflected[Band::ThermalIr] * 5.0,
            "at ten microns a giant is its own light: {} against {}",
            emitted[Band::ThermalIr],
            reflected[Band::ThermalIr],
        );
        assert!(
            reflected[Band::V] > emitted[Band::V] * 1.0e6,
            "in the visible it is a mirror: {} against {}",
            reflected[Band::V],
            emitted[Band::V],
        );
    }

    /// Internal heat is what makes it work, not the albedo correction alone.
    ///
    /// A giant with no heat of its own would sit at the temperature the sunlight it keeps
    /// leaves it at, and at ten microns the difference between that and 124 K is most of a
    /// factor of two.
    #[test]
    fn a_giants_own_heat_reaches_the_screen() {
        let jupiter = giant();
        let mut inert = jupiter.clone();
        // The same body, absorbing and re-emitting and nothing else.
        inert.effective_k =
            jupiter.equilibrium_k * (1.0 - Surface::GasGiant.bond_albedo()).powf(0.25);

        let warm = emitted_radiance(&jupiter)[Band::ThermalIr];
        let cold = emitted_radiance(&inert)[Band::ThermalIr];
        assert!(jupiter.effective_k > inert.effective_k, "internal heat should warm it");
        assert!(warm > cold * 1.5, "and show: {warm} against {cold}");
    }

    /// The two ways a body can be drawn have to radiate at the same temperature, or a planet
    /// changes brightness as it crosses the resolution threshold.
    ///
    /// Flux is radiance times solid angle, so the resolved surface's own emission scaled by the
    /// disc it covers must be the unresolved point's thermal term. Only the thermal halves are
    /// compared: the reflected half goes through `effective_radius`, which is a different
    /// fiction on purpose.
    #[test]
    fn the_resolved_and_unresolved_paths_glow_alike() {
        let jupiter = giant();
        let observer = DVec3::X * 3.0e-5;
        let distance_m = observer.distance(jupiter.position_ly) * M_PER_LY;

        let surface = emitted_radiance(&jupiter);
        let disc = std::f64::consts::PI * (jupiter.radius_m / distance_m).powi(2);
        let point = crate::session::bare(jupiter.effective_k, jupiter.radius_m, distance_m);

        for band in [Band::ThermalIr, Band::K, Band::Radio] {
            let from_surface = surface[band] as f64 * disc;
            let from_point = point[band] as f64;
            assert!(
                (from_surface / from_point - 1.0).abs() < 0.02,
                "{band:?}: {from_surface:e} resolved against {from_point:e} unresolved",
            );
        }
    }

    /// The crossover: a body is a point until it has a shape worth drawing.
    #[test]
    fn a_body_resolves_only_once_it_is_more_than_a_few_pixels() {
        let rad_per_px = crate::starfield::radians_per_pixel(std::f32::consts::FRAC_PI_2, 1080.0);
        let radius = 6.0e7;
        // The distance at which it subtends exactly the threshold.
        let edge = radius / (RESOLVE_PX as f64 * rad_per_px as f64) / M_PER_LY;
        assert!(is_resolved(&body(radius, DVec3::X * edge * 0.5), DVec3::ZERO, rad_per_px));
        assert!(!is_resolved(&body(radius, DVec3::X * edge * 2.0), DVec3::ZERO, rad_per_px));
    }

    #[test]
    fn nothing_resolves_without_a_camera() {
        assert!(!is_resolved(&body(6.0e7, DVec3::X * 1e-6), DVec3::ZERO, 0.0));
    }

    /// A resolved body sits in the same exposure as the sky behind it, so its level comes from
    /// the session's own tone map rather than from a constant.
    ///
    /// Exposed here for a surface, because the star-field exposure puts every lit body at the
    /// top of the window and the ordering would be invisible.
    fn v(r: PerBand<f32>) -> f64 {
        r[em_spectra::Band::V] as f64
    }

    /// `reflected_radiance`, not `surface_radiance`: only the half that comes from the star
    /// obeys the star's inverse square. A body's own emission does not move when it does, which
    /// is the point of separating them.
    #[test]
    fn a_lit_surface_dims_as_the_inverse_square_of_its_distance_from_the_star() {
        let b = body(6.0e7, DVec3::ZERO);
        let near = v(reflected_radiance(&b, 6.957e8, 5772.0, AU));
        let far = v(reflected_radiance(&b, 6.957e8, 5772.0, 30.0 * AU));
        assert!((near / far - 900.0).abs() / 900.0 < 1e-6, "{near} against {far}");
        assert_eq!(v(reflected_radiance(&b, 6.957e8, 5772.0, 0.0)), 0.0);

        // And the emission does not move at all.
        let own = v(emitted_radiance(&b));
        assert_eq!(own, v(emitted_radiance(&b)));
        assert!(own > 0.0, "a warm body is never perfectly dark");
    }

    /// Ice reflects six times what bare rock does, and the surface class is what knows it.
    #[test]
    fn a_brighter_surface_emits_more() {
        let mut icy = body(6.0e5, DVec3::ZERO);
        icy.surface = Surface::Ice;
        let rocky = body(6.0e5, DVec3::ZERO);
        let at = |b: &Drawable| v(surface_radiance(b, 6.957e8, 5772.0, 5.0 * AU));
        assert!(at(&icy) > at(&rocky) * 4.0, "{} against {}", at(&icy), at(&rocky));
    }

    /// The cheap ranking must agree with the spectrum it stands in for, or truncating to the
    /// brightest thirty-two would throw away one that counted. Checked across the split it is
    /// most likely to get wrong: a cold body large enough that its own heat outweighs what it
    /// reflects, against a small bright one.
    #[test]
    fn the_ranking_proxy_orders_bodies_the_way_their_spectra_do() {
        let mut candidates = Vec::new();
        for (radius, effective, temperature) in [
            (6.0e7, 3.0e4, 90.0),
            (6.0e5, 1.0e3, 40.0),
            (2.0e6, 2.0e2, 700.0),
            (1.0e7, 9.0e3, 160.0),
        ] {
            let mut b = body(radius, DVec3::X * 1.0e-5);
            b.effective_radius_m = effective;
            b.equilibrium_k = temperature;
            b.effective_k = temperature;
            candidates.push(b);
        }
        let luminance = |b: &Drawable| {
            let f = point_flux(b, 5772.0, DVec3::ZERO);
            Band::ALL.iter().map(|x| f[*x] as f64).sum::<f64>()
        };
        let mut by_proxy: Vec<usize> = (0..candidates.len()).collect();
        let mut by_spectrum = by_proxy.clone();
        by_proxy.sort_by(|a, b| {
            point_power(&candidates[*b], 5772.0, DVec3::ZERO)
                .total_cmp(&point_power(&candidates[*a], 5772.0, DVec3::ZERO))
        });
        by_spectrum.sort_by(|a, b| luminance(&candidates[*b]).total_cmp(&luminance(&candidates[*a])));
        assert_eq!(by_proxy, by_spectrum);
    }

    /// The displayed window is five stops wide and a lit surface's range is tens, so
    /// the level clips. Which end it clips at is the exposure's business, not this function's.
    #[test]
    fn the_level_is_a_window_and_a_close_surface_fills_it() {
        let mut session = Session::new(&AuthoredStars::sample(), 3);
        session.tone.surface_reference = 2.0;
        let b = body(6.0e7, DVec3::ZERO);
        let near = surface_level(&session, &b, 6.957e8, 5772.0, AU);
        let far = surface_level(&session, &b, 6.957e8, 5772.0, 30.0 * AU);
        assert_eq!(near, 1.0, "a close surface is at the top of the window");
        assert_eq!(far, 0.0, "and one ten stops down is under it");
        assert_eq!(surface_level(&session, &b, 6.957e8, 5772.0, 0.0), 0.0);
    }

    /// The point of metering the bodies: a planet large enough to be the picture is exposed
    /// for, and the same planet at a different distance from its star is not the same color.
    ///
    /// A body's surface radiance does not change as the ship approaches it — only its size on
    /// screen does — so this is a statement about the reference following the subject.
    #[test]
    fn a_metered_planet_lands_inside_the_window_and_a_dimmer_one_below_it() {
        let mut session = Session::new(&AuthoredStars::sample(), 3);
        let inner = body(6.0e7, DVec3::ZERO);
        let outer = body(6.0e7, DVec3::ZERO);
        let radiance = |d: f64| surface_radiance(&inner, 6.957e8, 5772.0, d);
        // One planet at one astronomical unit, filling a good part of the frame.
        session.scene = Scene {
            point_sr: NOMINAL_POINT_SR,
            points: Vec::new(),
            discs: vec![Disc { radiance: radiance(AU), solid_angle_sr: 0.2 }],
        };
        session.auto_expose();
        let near = surface_level(&session, &inner, 6.957e8, 5772.0, AU);
        let far = surface_level(&session, &outer, 6.957e8, 5772.0, 4.0 * AU);
        assert!(near > 0.9, "the metered subject is at the top of the window, not past it: {near}");
        assert!(far < near, "four times as far is four stops down: {far} against {near}");
    }

}
