//! What a burn looks like: how big the exhaust is, and how hot.
//!
//! One number decides all of it — the jet power, `½ F v`, which [`lc_world::flight::Drive`]
//! works out from what is being pushed and how hard. A heavier ship or a harder burn is a
//! longer, hotter plume, and that is not a rule imposed here, it is what more power in the same
//! nozzle means.
//!
//! **The shape is a display model and the light is not.** How long the cone is drawn and how
//! far it flares are choices, made here and tunable; the temperature is then forced, because
//! the power has to go somewhere and a blackbody of that area at that temperature is the only
//! surface that radiates it. So a plume's colour is a consequence rather than a setting, and a
//! fifty-kilometre ship's drive comes out blue-white next to a tug's orange without anyone
//! choosing that.

use bevy::prelude::*;
use em_render::plume_material::{PlumeMaterial, PlumeUniform};
use em_render::render_space::sim_to_render;
use em_spectra::{Band, PerBand, blackbody};
use glam::DVec3;
use lc_proto::ShipId;

use crate::session::Session;
use crate::system::{M_PER_LY, UNIT_M};

/// The Stefan-Boltzmann constant, watts per square metre per kelvin to the fourth.
pub const SIGMA: f64 = 5.670_374_419e-8;

/// How long a plume is at the drive's own rated acceleration, in hull lengths.
///
/// A choice, and the main one. Long enough to read as a torch, and short enough to fit beside
/// the ship at the zoom the camera opens at — which is framed on the hull, so a plume of three
/// lengths was most of the screen and the ship was a detail in the corner of its own exhaust.
pub const LENGTHS_AT_RATED: f64 = 1.5;

/// How the length answers to being throttled, as an exponent on the fraction of rated thrust.
///
/// A half, so a quarter-thrust burn is half the plume. Linear made a gentle correction
/// invisible and a hard burn no more impressive than a moderate one.
pub const LENGTH_EXPONENT: f64 = 0.5;

/// The nozzle's radius, as a fraction of the hull's beam.
pub const THROAT_OF_BEAM: f64 = 0.35;

/// How much wider the gas is where it ends than where it leaves the nozzle.
///
/// The expansion ratio, and the other shape knob. A plume in vacuum has no ambient pressure to
/// hold it in, so it keeps spreading — this is how far it gets by the time it has faded.
pub const EXPANSION: f64 = 5.0;

/// How much wider the proxy is than the gas, so the feathered edge has somewhere to be drawn.
pub const MARGIN: f64 = 1.3;

/// How sharply the density falls off across the plume, and how fast it thins along it.
pub const EDGE: f32 = 2.5;
pub const TAPER: f32 = 1.5;

/// Radial and lengthwise divisions of the shared proxy.
pub const SIDES: u32 = 24;

/// How far above the exposure's reference the core of a plume sits, in stops.
///
/// **The one number that is not physics.** A plume's colour is forced — see
/// [`temperature_k`] — but its *brightness* is not, because the gas is optically thin by an
/// amount nothing here models: what reaches the eye is a fraction of the blackbody radiance,
/// and that fraction is the fudge. Four stops over the reference puts the core past the top of
/// a two-and-a-half stop window, so the middle clips and blooms like something that hot should,
/// while the falloff carries the edges back down through the window and the cone has a shape.
///
/// Left as a multiple of the reference rather than an absolute, so it holds when the exposure
/// moves. Set it from the colour instead and a hot plume is a white rectangle: a blackbody at
/// fifty thousand kelvin is ten decades over a planet, and there is no window that holds both.
pub const CORE_STOPS: f64 = 4.0;

/// One craft's exhaust. `None` is the player's own ship, matching [`crate::hull::Hull`].
#[derive(Component)]
pub struct Plume(pub Option<ShipId>);

/// The shared proxy, and which craft currently have a plume.
#[derive(Resource, Default)]
pub struct Plumes {
    proxy: Option<Handle<Mesh>>,
    drawn: Vec<Option<ShipId>>,
}

/// How long the exhaust runs and how wide it is at each end, metres.
///
/// The length answers to the throttle and the width does not: a nozzle is a nozzle whatever is
/// going through it, and what changes when a drive is pushed is how far the gas gets before it
/// has spent itself.
pub fn extent(length_m: f64, throttle: f64) -> (f64, f64, f64) {
    let long = length_m * LENGTHS_AT_RATED * throttle.clamp(0.0, 4.0).powf(LENGTH_EXPONENT);
    let throat = length_m * lc_world::craft::BEAM_PER_LENGTH * 0.5 * THROAT_OF_BEAM;
    (long, throat, throat * EXPANSION)
}

/// The lateral area of the cone the gas fills, square metres.
///
/// What the power has to radiate through, which is what sets the temperature. A frustum's
/// slant surface: `π (r0 + r1) √(L² + (r1 − r0)²)`.
pub fn radiating_area_m2(long_m: f64, throat_m: f64, mouth_m: f64) -> f64 {
    let flare = mouth_m - throat_m;
    std::f64::consts::PI * (throat_m + mouth_m) * (long_m * long_m + flare * flare).sqrt()
}

/// How hot a plume of that size has to be to carry that power away, kelvin.
///
/// Stefan-Boltzmann, inverted. Nothing is being fitted here: the drive makes `power_w`, the gas
/// is the only thing to carry it, and a blackbody of this area radiating that much has exactly
/// one temperature. Which is why the colour cannot be set — push a bigger ship harder and the
/// plume goes blue whether or not anybody wanted it to.
pub fn temperature_k(power_w: f64, area_m2: f64) -> f64 {
    if power_w <= 0.0 || area_m2 <= 0.0 {
        return 0.0;
    }
    (power_w / (SIGMA * area_m2)).powf(0.25)
}

/// A craft with its drive lit, reduced to what the proxy needs.
struct Burning {
    /// From the eye, in simulation axes, metres.
    offset_m: DVec3,
    /// The hull's own length, so the nozzle can be put at its tail.
    hull_m: f64,
    /// Which way the nose points, so the exhaust can go the other way.
    facing: DVec3,
    long_m: f64,
    throat_m: f64,
    mouth_m: f64,
    power_w: f64,
}

impl Burning {
    fn of(length_m: f64, power_w: f64, rated_w: f64, facing: DVec3, offset_m: DVec3) -> Option<Self> {
        if power_w <= 0.0 || facing == DVec3::ZERO {
            return None;
        }
        // Against what this ship's own drive would make at its rating, so a tug at full thrust
        // gets a full plume and is not measured against a warship's.
        let throttle = if rated_w > 0.0 { power_w / rated_w } else { 1.0 };
        let (long_m, throat_m, mouth_m) = extent(length_m, throttle);
        Some(Self { offset_m, hull_m: length_m, facing, long_m, throat_m, mouth_m, power_w })
    }
}

/// Everything with its drive lit this frame.
fn burning(game: &Session, uplink: &crate::uplink::Uplink, eye: &crate::hull::Eye, look: DVec3) -> Vec<(Option<ShipId>, Burning)> {
    let now = game.coordinate_time_s();
    let mut out = Vec::new();
    let mine = &game.ship;
    let rated = mine.motion.drive.jet_power_w(mine.mass_kg(), mine.motion.drive.accel_g);
    if let Some(lit) = Burning::of(
        mine.length_m,
        mine.jet_power_w(now),
        rated,
        mine.facing_at(now).unwrap_or(look),
        look * eye.boom_m,
    ) {
        out.push((None, lit));
    }
    for contact in &uplink.contacts {
        // A contact's rating is not known — only what it is doing — so its own burn is taken
        // as full. The cost is that a ship seen easing off looks like a smaller ship at full
        // thrust, which is a thing an observer genuinely cannot tell apart.
        let power = contact.jet_power_w;
        if let Some(lit) = Burning::of(
            contact.length_m,
            power,
            power,
            contact.facing,
            (contact.position_ly - eye.at_ly) * M_PER_LY,
        ) {
            out.push((Some(contact.ship_id), lit));
        }
    }
    out
}

/// The proxy: a closed cylinder wide enough to hold the gas, with its axis on `+y`.
fn proxy() -> Mesh {
    Cylinder::new(1.0, 1.0).mesh().resolution(SIDES).segments(1).build()
}

/// The rotation putting the proxy's `+y` down the exhaust, which is aft of the nose.
fn along_exhaust(facing: DVec3) -> Quat {
    let aft = sim_to_render(-facing.normalize_or_zero()).as_vec3();
    Quat::from_rotation_arc(Vec3::Y, aft)
}

fn uniforms(lit: &Burning, session: &Session, eye_local: Vec3) -> PlumeUniform {
    let area = radiating_area_m2(lit.long_m, lit.throat_m, lit.mouth_m);
    let kelvin = temperature_k(lit.power_w, area);
    let radiance = PerBand::new(std::array::from_fn(|i| {
        blackbody::band_radiance(Band::ALL[i], kelvin) as f32
    }));
    let glow = Vec3::from_array(session.mapping.apply(&radiance));
    // Scaled so the core lands where [`CORE_STOPS`] says, whatever the colour came out as.
    let luminance = glow.dot(Vec3::new(0.2126, 0.7152, 0.0722)) as f64;
    let scale = if luminance > 0.0 {
        session.tone.surface_reference as f64 * 2f64.powf(CORE_STOPS) / luminance
    } else {
        0.0
    };
    let wall = lit.mouth_m * MARGIN;
    PlumeUniform {
        glow: glow.extend(0.0),
        shape: Vec4::new(
            (lit.throat_m / wall) as f32,
            (lit.mouth_m / wall) as f32,
            EDGE,
            TAPER,
        ),
        eye_local: eye_local.extend(0.0),
        // The march sums a density with no units, so the brightness is a scale rather than a
        // measurement — the *colour* is the physics and this only says how much of it there is.
        exposure: Vec4::new(
            session.tone.surface_reference,
            session.tone.stops,
            scale as f32,
            0.0,
        ),
    }
}

/// Keep a proxy for every craft with its drive lit, and take it away when the drive goes out.
pub fn update_plumes(
    mut commands: Commands,
    game: Res<crate::app::Game>,
    ui: Res<crate::app::Ui>,
    uplink: Res<crate::uplink::Uplink>,
    eye: Res<crate::hull::Eye>,
    mut plumes: ResMut<Plumes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PlumeMaterial>>,
    existing: Query<(Entity, &Plume)>,
    mut placed: Query<(&mut Transform, &MeshMaterial3d<PlumeMaterial>, &Plume)>,
) {
    let look = ui.look.forward();
    let want = burning(&game.0, &uplink, &eye, look);
    let keys: Vec<Option<ShipId>> = want.iter().map(|(id, _)| *id).collect();

    if keys != plumes.drawn {
        for (entity, _) in &existing {
            commands.entity(entity).despawn();
        }
        let proxy = plumes.proxy.get_or_insert_with(|| meshes.add(proxy())).clone();
        for (id, _) in &want {
            commands.spawn((
                Mesh3d(proxy.clone()),
                MeshMaterial3d(materials.add(PlumeMaterial::default())),
                Transform::default(),
                // Placed by hand at a scale where the mesh's own bounds say nothing about
                // where it lands, exactly as a hull is.
                bevy::camera::visibility::NoFrustumCulling,
                Plume(*id),
            ));
        }
        plumes.drawn = keys;
        // Placed next frame, when the spawns exist. One frame at the origin is one frame with
        // a plume inside the camera.
        return;
    }

    for (mut transform, material, marker) in placed.iter_mut() {
        let Some((_, lit)) = want.iter().find(|(id, _)| *id == marker.0) else { continue };
        let wall = lit.mouth_m * MARGIN;
        // The nozzle is at the hull's tail — half a hull aft of its centre — and the proxy's
        // own centre is half a plume further aft again. Measuring from the hull's centre put
        // the gas half inside the ship.
        let tail = lit.offset_m
            - lit.facing * (lit.hull_m * 0.5 + lit.long_m * 0.5);
        transform.translation = sim_to_render(tail / UNIT_M).as_vec3();
        transform.rotation = along_exhaust(lit.facing);
        transform.scale = Vec3::new(
            (wall / UNIT_M) as f32,
            (lit.long_m / UNIT_M) as f32,
            (wall / UNIT_M) as f32,
        );

        let Some(asset) = materials.get_mut(&material.0) else { continue };
        // The eye is at the render origin, so where it sits in the proxy's own space is the
        // transform undone. The march needs it there and nowhere else.
        let eye_local = transform.to_matrix().inverse().transform_point3(Vec3::ZERO);
        let next = uniforms(lit, &game.0, eye_local);
        if asset.uniforms != next {
            asset.uniforms = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIP_M: f64 = 500.0;

    /// A harder burn is a longer plume, and an unlit drive has none at all.
    #[test]
    fn the_length_answers_to_the_throttle() {
        let (full, _, _) = extent(SHIP_M, 1.0);
        let (quarter, _, _) = extent(SHIP_M, 0.25);
        assert!((quarter / full - 0.5).abs() < 1.0e-9, "a quarter thrust is half the plume");
        assert_eq!(extent(SHIP_M, 0.0).0, 0.0);
        // And it is a plume rather than a wisp or a tail: a few hull lengths.
        assert!(full > SHIP_M && full < SHIP_M * 10.0, "{full} m off a {SHIP_M} m ship");
    }

    /// The nozzle does not change size when the drive is throttled; only the gas goes further.
    #[test]
    fn the_nozzle_is_the_same_nozzle_at_any_thrust() {
        let (_, throat, mouth) = extent(SHIP_M, 1.0);
        let (_, quiet_throat, quiet_mouth) = extent(SHIP_M, 0.1);
        assert_eq!(throat, quiet_throat);
        assert_eq!(mouth, quiet_mouth);
        assert!((mouth / throat - EXPANSION).abs() < 1.0e-9, "that is the expansion ratio");
    }

    /// **The colour is forced, not chosen.** The power has to go somewhere, and a blackbody of
    /// that area radiating it has exactly one temperature.
    #[test]
    fn a_bigger_ship_burns_hotter_without_anyone_deciding_to() {
        let at = |length_m: f64| {
            let power = lc_world::flight::Drive::DEFAULT.jet_power_w(mass_of(length_m), 5.0);
            let (long, throat, mouth) = extent(length_m, 1.0);
            temperature_k(power, radiating_area_m2(long, throat, mouth))
        };
        let small = at(500.0);
        let large = at(50_000.0);
        assert!(large > small * 2.0, "{small} K against {large} K");
        // Both in a range that reads as fire rather than as a light bulb or a nuclear weapon.
        assert!(small > 3_000.0 && small < 1.0e5, "{small} K");
        assert!(large > 3_000.0 && large < 1.0e6, "{large} K");
    }

    /// Harder thrust is hotter too, for the same reason: more power through a nozzle that grew
    /// only in length.
    #[test]
    fn a_harder_burn_is_a_hotter_one() {
        let at = |accel_g: f64| {
            let power = lc_world::flight::Drive::DEFAULT.jet_power_w(mass_of(SHIP_M), accel_g);
            let (long, throat, mouth) = extent(SHIP_M, accel_g / 5.0);
            temperature_k(power, radiating_area_m2(long, throat, mouth))
        };
        assert!(at(20.0) > at(5.0));
        assert_eq!(at(0.0), 0.0, "an unlit drive is not a cold plume, it is no plume");
    }

    fn mass_of(length_m: f64) -> f64 {
        let mut craft = lc_world::craft::Craft::at(
            lc_world::craft::CraftId(1),
            lc_world::craft::Kind::Ship,
            DVec3::ZERO,
        );
        craft.length_m = length_m;
        craft.mass_kg()
    }

    /// The exhaust goes aft, which is the one thing about its direction that must never be
    /// wrong: a plume drawn forward is a ship visibly pushing itself backwards.
    #[test]
    fn the_exhaust_points_away_from_the_nose() {
        for facing in [DVec3::X, DVec3::Y, DVec3::new(1.0, -2.0, 0.5).normalize()] {
            let aft = along_exhaust(facing) * Vec3::Y;
            let want = sim_to_render(-facing.normalize()).as_vec3();
            assert!((aft - want).length() < 1.0e-6, "{facing} sent the exhaust to {aft}");
        }
    }
}
