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
//!
//! The streaks follow the same division. *That* the gas is uneven is physics — a drive burns
//! fuel-rich and the flow combs what leaves the injector unmixed into lanes — and so is what
//! those lanes radiate, which is a cooler greybody worked out here and handed over as a second
//! colour. How fast they travel is not: see [`CHURN_EXPONENT`].

use bevy::prelude::*;
use em_render::plume_material::{CHURN_PERIOD, PlumeMaterial, PlumeUniform};
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
/// a two-and-a-half stop window, so the middle overflows and blooms like something that hot
/// should — see [`OVERFLOW_GAIN`] for where the overflow goes — while the falloff carries the
/// edges back down through the window and the cone has a shape.
///
/// Left as a multiple of the reference rather than an absolute, so it holds when the exposure
/// moves. Set it from the colour instead and a hot plume is a white rectangle: a blackbody at
/// fifty thousand kelvin is ten decades over a planet, and there is no window that holds both.
pub const CORE_STOPS: f64 = 4.0;

/// What a stop past the top of the exposure window is worth as HDR value.
///
/// The core sits [`CORE_STOPS`] over the reference and the window is two and a half stops wide,
/// so most of the cone has nowhere left to go inside it. Clipped there, the whole column came
/// back as one flat lavender: the tone curve holds hue and saturation constant and scales only
/// the value, so a ray through the deep middle and a ray grazing the flank — which differ by
/// decades of column depth — drew the same colour, and the plume read as a cut-out rather than
/// as a volume.
///
/// Letting the overflow out as HDR is what the starfield already does with a star twenty stops
/// over, and for the same reason: the excess becomes a halo rather than a whiter white. The
/// display transform desaturates the middle toward white, bloom spreads it, and the thin edges
/// stay inside the window with their colour.
///
/// The sky uses a quarter. A plume wants an order more, and the case that decides it is
/// `thermal`: clipped, its core sat at a saturation of 0.37, and one stop of gain only brings
/// that to 0.33 — still a flat blue shape. Three brings it to 0.16 against a flank of 0.89,
/// which is a white-hot core in a coloured cone with the sooty lanes reading against it. Six
/// buys 0.09 and nothing else, every channel's peak already being at 255. A plume wants more
/// than the sky does because it is a near object filling a good part of the frame rather than a
/// point a few pixels across, so its overflow has somewhere to go.
pub const OVERFLOW_GAIN: f32 = 3.0;

/// Lattice cells across the cone's own radius, and along its whole length.
///
/// Their ratio is the aspect of a filament, and it wants to be lopsided: at anything near one
/// the noise reads as a dirty cloud hanging in the exhaust rather than as gas being drawn out.
/// Seven to one and a half puts about fourteen filaments across the plume, each running most of
/// its length. Fewer and the plume is draped rather than striated; many more and a ray crosses
/// enough of them to average them flat again, which is the failure this whole coordinate choice
/// exists to avoid.
pub const CHURN_ACROSS: f32 = 7.0;
pub const CHURN_ALONG: f32 = 1.5;

/// How much of the gas the streaks may claim.
pub const CHURN_BITE: f32 = 1.0;

/// How cool the fuel-rich gas runs, as a fraction of the core's temperature.
pub const SOOT_FRACTION: f64 = 0.6;

/// How much of a blackbody's output the soot actually manages.
///
/// The only place in this file something is *not* a blackbody, and it is the honest correction
/// rather than a fudge: soot is the one constituent of a plume that is optically thick, so it
/// radiates as a greybody. It also guarantees the streaks read. A temperature ratio alone does
/// not: at fifty thousand kelvin the visible band is on the Rayleigh-Jeans side, where radiance
/// goes as `T` and not as `T^4`, so a plume that hot would have shown streaks six per cent
/// darker than the gas around them and looked exactly as smooth as before.
pub const SOOT_EMISSIVITY: f32 = 0.25;

/// How far the churn travels in a second of real time while the clock runs at real time, in
/// plume lengths.
pub const TRAVERSES_AT_REAL_TIME: f64 = 0.12;

/// The root by which the clock's speed is compressed into the churn's.
///
/// The ladder in [`crate::ui::RATE_LADDER`] spans seven decades, from real time to a Julian year
/// a second. The band in which a moving pattern reads as *moving* — rather than as a still
/// picture at one end or as static at the other — spans well under one. An eighth root is what
/// maps the one onto the other: every rung is a visibly different churn, the top of the ladder
/// is about eight times the bottom, and no rung is a strobe.
///
/// **What it must not do is decouple.** The gas itself crosses the plume in milliseconds, so
/// there is no rung at which the true rate is anything but a blur, and drawing the churn at all
/// is already a display model. The one thing that has to be exact is the end of the range: a
/// stopped clock is a still plume, which is what every `--rate 0` photograph depends on.
pub const CHURN_EXPONENT: f64 = 0.125;

/// One craft's exhaust. `None` is the player's own ship, matching [`crate::hull::Hull`].
#[derive(Component)]
pub struct Plume(pub Option<ShipId>);

/// The shared proxy, which craft currently have a plume, and where the churn has got to.
#[derive(Resource, Default)]
pub struct Plumes {
    proxy: Option<Handle<Mesh>>,
    drawn: Vec<Option<ShipId>>,
    /// How far aft the streaks have travelled, in lattice cells, wrapped at [`CHURN_PERIOD`].
    phase: f64,
    /// The coordinate clock last frame. `None` until the first, which therefore advances by
    /// nothing rather than by however long the client spent loading.
    clock_s: Option<f64>,
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

/// How far the churn travels this frame, in plume lengths.
///
/// Measured in plume *lengths* rather than metres, so a fifty-kilometre ship's exhaust and a
/// tug's churn at the same rate on the screen. The camera frames on the hull, so that is the
/// comparison that matters; in metres per second the big one is a hundred times the faster,
/// which is also true.
pub fn churn_step(real_s: f64, simulated_s: f64) -> f64 {
    if real_s <= 0.0 || simulated_s <= 0.0 {
        return 0.0;
    }
    TRAVERSES_AT_REAL_TIME * (simulated_s / real_s).powf(CHURN_EXPONENT) * real_s
}

/// Where in the pattern a craft's plume starts, in lattice cells.
///
/// The noise repeats at [`CHURN_PERIOD`], so offsetting the phase is the whole of giving every
/// ship its own streaks — no second uniform, and two craft burning alongside each other do not
/// flicker in step.
pub fn seed(of: Option<ShipId>) -> f64 {
    // The player's own ship has no id of its own, so it takes one no id can collide with.
    let key = match of {
        None => u64::MAX,
        Some(ShipId(n)) => n as u64,
    };
    let mut h = key ^ 0x2545_f491_4f6c_dd1d;
    h = (h ^ (h >> 33)).wrapping_mul(0xff51_afd7_ed55_8ccd);
    h = (h ^ (h >> 33)).wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    (h >> 40) as f64 / (1u64 << 24) as f64 * CHURN_PERIOD as f64
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
        mine.facing_at(now).unwrap_or(DVec3::X),
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

/// What a blackbody at `kelvin` looks like through this observer's bands, linear display RGB.
fn shine(session: &Session, kelvin: f64) -> Vec3 {
    let radiance = PerBand::new(std::array::from_fn(|i| {
        blackbody::band_radiance(Band::ALL[i], kelvin) as f32
    }));
    Vec3::from_array(session.mapping.apply(&radiance))
}

fn uniforms(lit: &Burning, session: &Session, eye_local: Vec3, phase: f64) -> PlumeUniform {
    let area = radiating_area_m2(lit.long_m, lit.throat_m, lit.mouth_m);
    let kelvin = temperature_k(lit.power_w, area);
    let glow = shine(session, kelvin);
    // The same mapping at a lower temperature, so the streaks' colour is as forced as the
    // core's and the ratio between them is the physics rather than a tint.
    let soot = shine(session, kelvin * SOOT_FRACTION) * SOOT_EMISSIVITY;
    // Scaled so the core lands where [`CORE_STOPS`] says, whatever the colour came out as.
    // Against the clean gas, which is what the core is made of: the streaks are faded out
    // toward the axis, so calibrating against a mixture would move the exposure with the churn.
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
        soot: soot.extend(0.0),
        churn: Vec4::new(phase as f32, CHURN_ACROSS, CHURN_ALONG, CHURN_BITE),
        // The march sums a density with no units, so the brightness is a scale rather than a
        // measurement — the *colour* is the physics and this only says how much of it there is.
        exposure: Vec4::new(
            session.tone.surface_reference,
            session.tone.stops,
            scale as f32,
            OVERFLOW_GAIN,
        ),
    }
}

/// Keep a proxy for every craft with its drive lit, and take it away when the drive goes out.
pub fn update_plumes(
    mut commands: Commands,
    time: Res<Time>,
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

    // Taken from the clock itself rather than from the rate knob, so a correction from the
    // server moves the churn with everything else and the churn does not need to know who owns
    // the rate. Ahead of the early return below: a frame that respawns is still a frame.
    let now = game.0.coordinate_time_s();
    let simulated = plumes.clock_s.map_or(0.0, |was| now - was);
    plumes.clock_s = Some(now);
    let travelled = churn_step(time.delta_secs_f64(), simulated) * CHURN_ALONG as f64;
    plumes.phase = (plumes.phase + travelled).rem_euclid(CHURN_PERIOD as f64);

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
        let phase = (plumes.phase + seed(marker.0)).rem_euclid(CHURN_PERIOD as f64);
        let next = uniforms(lit, &game.0, eye_local, phase);
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

    /// The property every `--rate 0` photograph rests on. Two frames of a stopped clock are the
    /// same picture, and an hour of them is the same picture.
    #[test]
    fn a_stopped_clock_is_a_still_plume() {
        assert_eq!(churn_step(1.0 / 60.0, 0.0), 0.0);
        assert_eq!(churn_step(3600.0, 0.0), 0.0);
        // And a clock corrected *backwards* holds rather than running the plume in reverse.
        assert_eq!(churn_step(1.0 / 60.0, -5.0), 0.0);
    }

    /// Every rung of the ladder is a different churn, and the whole ladder is a small factor.
    ///
    /// Both halves matter. Without the first the clock is decoration; without the second the top
    /// of the ladder moves the pattern further than a streak between one frame and the next,
    /// which is not a fast plume, it is static.
    #[test]
    fn a_faster_clock_is_a_faster_churn_but_not_by_much() {
        let frame = 1.0 / 60.0;
        let at = |rung: f64| churn_step(frame, frame * rung * crate::session::TIME_RATE);
        let rungs: Vec<f64> = crate::ui::RATE_LADDER
            .iter()
            .map(|(r, _)| *r)
            .filter(|r| *r > 0.0)
            .collect();
        let steps: Vec<f64> = rungs.iter().map(|r| at(*r)).collect();
        assert!(steps.windows(2).all(|w| w[1] > w[0] * 1.05), "{steps:?}");
        let span = steps[steps.len() - 1] / steps[0];
        assert!(span > 4.0 && span < 20.0, "the ladder spans {span} in churn");
        // A streak is about `1 / CHURN_ALONG` of a lattice cell; crossing half of one in a frame
        // is where a moving pattern turns into a hissing one.
        let worst = steps[steps.len() - 1] * CHURN_ALONG as f64;
        assert!(worst < 0.5, "{worst} lattice cells in a frame");
    }

    /// Two ships burning side by side do not flicker in step.
    ///
    /// Consecutive ids are the case that matters, because that is what a shard hands out, and
    /// `ShipId(0)` is the one that catches a multiply with nothing mixed into it.
    #[test]
    fn every_craft_gets_its_own_streaks() {
        let period = CHURN_PERIOD as f64;
        let seeds: Vec<f64> =
            (0..64).map(|n| seed(Some(ShipId(n)))).chain([seed(None)]).collect();
        assert!(seeds.iter().all(|s| (0.0..period).contains(s)), "{seeds:?}");
        let mut sorted = seeds.clone();
        sorted.sort_by(f64::total_cmp);
        assert!(sorted.windows(2).all(|w| w[0] != w[1]), "two craft share a seed");
        // And spread over the period rather than clustered in a corner of it.
        for eighth in 0..8 {
            let low = period * eighth as f64 / 8.0;
            assert!(
                seeds.iter().any(|s| *s >= low && *s < low + period / 8.0),
                "nothing in the {eighth}th of the period",
            );
        }
    }

    /// The streaks are darker than the gas around them at *any* plume temperature.
    ///
    /// The bug this is here for: a temperature ratio on its own does not do it. Above about ten
    /// thousand kelvin the visible band is on the Rayleigh-Jeans side of the peak, radiance goes
    /// as `T` rather than as `T^4`, and a streak six per cent down is a plume with no streaks.
    /// [`SOOT_EMISSIVITY`] is what carries it there.
    #[test]
    fn a_streak_is_darker_than_the_gas_beside_it_however_hot_the_plume() {
        for kelvin in [2_000.0, 6_000.0, 50_000.0, 500_000.0] {
            let visible = |t: f64| blackbody::band_radiance(Band::ALL[2], t);
            let ratio = visible(kelvin * SOOT_FRACTION) / visible(kelvin)
                * SOOT_EMISSIVITY as f64;
            // A stop and a half down at the very least, which is a lane one can see.
            assert!(ratio < 0.35, "{kelvin} K: streaks at {ratio} of the core");
            assert!(ratio > 0.0, "{kelvin} K: streaks are not holes");
        }
    }

    /// Why the overflow is spent per channel rather than along one chroma.
    ///
    /// Under a natural mapping the plume's three channels are close enough that a single
    /// overflow along the chroma is nearly right. Under a false-colour one they are decades
    /// apart — ten microns, two microns and green are three quite different questions to ask a
    /// fifty-thousand-kelvin gas — and asking only the brightest of them reports its answer as
    /// the colour of all three. That is how the hottest object in the frame came back a flat
    /// saturated blue.
    ///
    /// This is the premise rather than the rendering, which no test can reach. If a preset is
    /// retuned until it fails, the shader's per-channel overflow is what to revisit.
    #[test]
    fn a_false_colour_mapping_pulls_the_plume_s_channels_decades_apart() {
        let radiance = PerBand::new(std::array::from_fn(|i| {
            blackbody::band_radiance(Band::ALL[i], 50_000.0) as f32
        }));
        let spread = |mapping: &em_spectra::BandMapping| {
            let rgb = mapping.apply(&radiance);
            let (low, high) = rgb.iter().fold((f32::MAX, 0.0f32), |(l, h), c| (l.min(*c), h.max(*c)));
            // Stops between the dimmest channel and the brightest.
            (high / low.max(1e-30)).log2()
        };
        let natural = spread(&em_spectra::presets::natural());
        let thermal = spread(&em_spectra::presets::thermal());
        assert!(natural < 2.0, "natural spreads the channels {natural} stops");
        assert!(thermal > 3.0, "thermal spreads them only {thermal} stops");
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
