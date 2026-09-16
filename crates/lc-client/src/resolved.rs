//! Bodies close enough to be discs rather than points.
//!
//! The point passes draw a body at its *effective* radius, which is a photometric fiction: the
//! radius a blackbody would need to deliver the same flux. That is right for anything
//! unresolved and wrong the moment a body is close enough to have a shape, which is why
//! Saturn's rings had nothing in the middle of them.
//!
//! A resolved body is a sphere instead, with a generated surface and a real terminator. The
//! crossover is angular: past a few pixels the disc is drawn, below it the point is.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use em_render::body_surface_material::{BodySurfaceMaterial, BodySurfaceUniform};
use em_render::render_space::sim_to_render;
use em_spectra::{Band, PerBand, blackbody};
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

#[derive(Component)]
pub struct ResolvedBody {
    pub name: String,
}

/// The shared unit sphere, and which bodies currently have one.
#[derive(Resource, Default)]
pub struct Resolved {
    pub mesh: Option<Handle<Mesh>>,
    pub drawn: Vec<String>,
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
/// Not every frame: re-placing costs a pass over every star in the catalogue. A quarter of a
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
    camera: Query<(&Projection, &Camera), With<Camera3d>>,
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
    let hulls = std::iter::once((game.ship.length_m, eye.boom_m, observer))
        .chain(uplink.contacts.iter().map(|c| {
            (c.length_m, c.position_ly.distance(observer) * M_PER_LY, c.position_ly)
        }));
    for (length_m, distance_m, at_ly) in hulls {
        scene.discs.push(Disc {
            radiance: crate::hull::radiance_at(&game.0, at_ly),
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
    let emitted = emitted_radiance(body);
    PerBand::new(std::array::from_fn(|i| reflected[Band::ALL[i]] + emitted[Band::ALL[i]]))
}

/// The starlight half: `p (R_star / d)^2 B(T_star)`, which has the star's spectrum.
pub fn reflected_radiance(
    body: &Drawable,
    star_radius_m: f64,
    star_teff_k: f64,
    star_distance_m: f64,
) -> PerBand<f32> {
    lit_radiance(body.surface.albedo(), star_radius_m, star_teff_k, star_distance_m)
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
    if body.effective_k <= 0.0 {
        return PerBand::splat(0.0);
    }
    PerBand::new(std::array::from_fn(|i| {
        blackbody::band_radiance(Band::ALL[i], body.effective_k) as f32
    }))
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
    star_radius_m: f64,
    star_teff_k: f64,
    star_distance_m: f64,
) -> (glam::Vec3, glam::Vec3) {
    let reflected = reflected_radiance(body, star_radius_m, star_teff_k, star_distance_m);
    let emitted = emitted_radiance(body);
    let through = |r| glam::Vec3::from_array(session.mapping.apply(&r));
    (through(reflected), through(emitted))
}

fn uniforms(
    body: &Drawable,
    star_ly: DVec3,
    tone: &crate::tonemap::ToneMap,
    reflected: glam::Vec3,
    emitted: glam::Vec3,
) -> BodySurfaceUniform {
    let (dark, light, contrast) = body.surface.palette();
    let to_star = sim_to_render((star_ly - body.position_ly).normalize_or_zero()).as_vec3();
    // Stable per body and independent of everything else, so a world looks the same every time
    // it is approached.
    let seed = (crate::system::name_seed(&body.name) % 100_000) as f32 * 1.0e-2;
    BodySurfaceUniform {
        dark: Vec4::new(dark[0], dark[1], dark[2], 1.0),
        light: Vec4::new(light[0], light[1], light[2], 1.0),
        to_star: to_star.extend(NIGHT),
        params: Vec4::new(0.0, contrast, seed, if body.surface.is_banded() { 1.0 } else { 0.0 }),
        reflected: reflected.extend(0.0),
        // `w` is how far the pattern inverts in the body's own light. See [`INVERSION`].
        emitted: emitted.extend(if body.surface.is_banded() { INVERSION } else { 0.0 }),
        exposure: Vec4::new(tone.surface_reference, tone.stops, 0.0, 0.0),
    }
}

/// Keep a sphere for every body close enough to be one.
pub fn update_resolved(
    mut commands: Commands,
    session: Res<crate::app::Game>,
    bodies: Res<crate::starfield::Bodies>,
    eye: Res<crate::hull::Eye>,
    mut resolved: ResMut<Resolved>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodySurfaceMaterial>>,
    camera: Query<(&Projection, &Camera), With<Camera3d>>,
    existing: Query<(Entity, &ResolvedBody)>,
    mut placed: Query<(&mut Transform, &MeshMaterial3d<BodySurfaceMaterial>, &ResolvedBody)>,
) {
    let rad_per_px = crate::starfield::camera_scale(&camera);
    let Some(system) = session.0.system.as_ref() else {
        for (entity, _) in &existing {
            commands.entity(entity).despawn();
        }
        resolved.drawn.clear();
        return;
    };
    let star_ly = system.star_position_ly();
    let (star_radius, star_teff) = (system.star_radius_m(), system.star_teff_k());

    let want: Vec<&Drawable> = bodies
        .drawn
        .iter()
        .filter(|d| is_resolved(d, eye.at_ly, rad_per_px))
        .collect();
    let names: Vec<String> = want.iter().map(|d| d.name.clone()).collect();

    if names != resolved.drawn {
        for (entity, _) in &existing {
            commands.entity(entity).despawn();
        }
        let mesh = resolved
            .mesh
            .get_or_insert_with(|| meshes.add(Sphere::new(1.0).mesh().uv(LONGITUDES, LATITUDES)))
            .clone();
        for body in &want {
            let star_distance = star_ly.distance(body.position_ly) * M_PER_LY;
            let (reflected, emitted) =
                surface_shading(&session.0, body, star_radius, star_teff, star_distance);
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(materials.add(BodySurfaceMaterial {
                    uniforms: uniforms(body, star_ly, &session.tone, reflected, emitted),
                })),
                Transform::default(),
                NoFrustumCulling,
                ResolvedBody { name: body.name.clone() },
            ));
        }
        resolved.drawn = names;
        return;
    }

    for (mut transform, material, marker) in placed.iter_mut() {
        let Some(body) = bodies.drawn.iter().find(|d| d.name == marker.name) else { continue };
        transform.translation =
            sim_to_render((body.position_ly - eye.at_ly) * M_PER_LY / UNIT_M).as_vec3();
        transform.rotation =
            Quat::from_rotation_arc(Vec3::Y, sim_to_render(body.pole).as_vec3().normalize());
        transform.scale = Vec3::splat((body.radius_m / UNIT_M) as f32);

        if let Some(asset) = materials.get_mut(&material.0) {
            let star_distance = star_ly.distance(body.position_ly) * M_PER_LY;
            let (reflected, emitted) =
                surface_shading(&session.0, body, star_radius, star_teff, star_distance);
            let next = uniforms(body, star_ly, &session.tone, reflected, emitted);
            if asset.uniforms != next {
                asset.uniforms = next;
            }
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
            name: "test".into(),
            rings: None,
            surface: Surface::Rock,
            pole: DVec3::Z,
            position_ly: at,
            radius_m,
            effective_radius_m: 1.0,
            equilibrium_k: 250.0,
            effective_k: Surface::Rock.effective_temperature(250.0),
        }
    }

    /// A Jupiter-like body at Jupiter's distance, so the numbers mean something.
    fn giant() -> Drawable {
        let mut b = body(6.99e7, DVec3::X * 1.0e-9);
        b.surface = Surface::GasGiant;
        // The grey balance at 5.2 AU, which is what `equilibrium_temperature` would give.
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

    /// The displayed window is two and a half stops wide and a lit surface's range is tens, so
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
    /// for, and the same planet at a different distance from its star is not the same colour.
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
