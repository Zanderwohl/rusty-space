//! Craft as things with a shape, and the camera that looks at one from outside.
//!
//! Two jobs that belong together because they are the same geometry read twice. A hull is an
//! ovoid five long by three across by one deep — see [`lc_world::craft`] — pointed along
//! [`lc_world::motion::facing`], and how far the orbit camera stands off is that same length
//! divided by an angle. Put them in separate modules and the ship's size means one thing to
//! the mesh and another to the zoom.
//!
//! **The camera still does not translate.** What moves is the origin everything is drawn
//! relative to: [`Eye`] is where the player is looking *from*, which is a boom's length behind
//! the hull rather than inside it, and the ship becomes the one thing drawn at an offset from
//! the render origin. Every other pass reads the eye where it used to read the ship, so the
//! parallax a ten-thousand-kilometer boom opens up against a nearby moon is simply correct
//! instead of being an error nobody measured.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use em_render::body_surface_material::{BodySurfaceMaterial, BodySurfaceUniform};
use em_render::render_space::sim_to_render;
use em_spectra::PerBand;
use glam::DVec3;
use lc_proto::ShipId;
use em_spectra::blackbody;
use lc_world::craft::{BEAM_PER_LENGTH, HEIGHT_PER_LENGTH, HULL_K};

use crate::session::Session;
use crate::system::{M_PER_LY, UNIT_M};
use crate::uplink::Uplink;

/// Longitude and latitude divisions of the shared ovoid.
///
/// Fewer than a planet's, because a hull is never the whole screen for long and its silhouette
/// is a smooth curve rather than a limb against a star field.
pub const LONGITUDES: u32 = 48;
pub const LATITUDES: u32 = 24;

/// The smallest a hull is allowed to be drawn before the camera stops backing away, in pixels
/// across its long axis.
///
/// Below about this a shape is a smudge, and zooming further out reads as the ship vanishing
/// rather than as distance.
pub const MIN_HULL_PX: f64 = 5.0;

/// How far the orbit camera starts, in hull lengths.
pub const DEFAULT_BOOM_LENGTHS: f64 = 4.0;

/// What one notch of the wheel multiplies the boom by.
pub const ZOOM_STEP: f64 = 1.25;

/// A hull's albedo. Gray paint, near enough, and the one number that says how bright a ship is
/// against the planet behind it.
pub const ALBEDO: f64 = 0.35;

/// The two ends of the palette. Equal, because a hull has no generated surface: the material
/// is a planet's and the pattern is turned off by a contrast of zero.
const GRAY: Vec4 = Vec4::new(0.30, 0.31, 0.33, 1.0);

/// Light on the unlit side, as a fraction. Higher than a planet's, because a hull is small
/// enough that its dark half is most of its silhouette and a hard terminator eats the shape.
const NIGHT: f32 = 0.10;

/// Where the player is looking from, and how far back that is.
///
/// Recomputed at the head of every frame and read by everything that turns a world position
/// into a render one. The ship's own position stays what the *physics* is about — light delay,
/// exposure, the read-out — and this is what the *picture* is about.
#[derive(Resource, Default)]
pub struct Eye {
    /// Light-years from the world origin.
    pub at_ly: DVec3,
    /// Meters from the hull's center, back along the view.
    pub boom_m: f64,
    /// The craft the boom is on, or `None` for the player's own.
    ///
    /// Read by [`drawn`], which gives that one craft the boom exactly rather than the
    /// difference of two light-year positions. A meters-wide offset taken that way loses most
    /// of its bits, and the hull the camera is closest to is the one that can least afford it.
    pub anchored: Option<ShipId>,
}

/// Which craft a hull stands for; `None` is the player's own.
#[derive(Component)]
pub struct Hull(pub Option<ShipId>);

/// The shared ovoid.
///
/// No remembered attitudes any more. A craft carries its own now — see
/// `lc_world::motion::facing_at` — so a ship keeps the nose its last order left it with,
/// turning toward the next one at the rate its hull allows, and the renderer only has to ask.
#[derive(Resource, Default)]
pub struct Hulls {
    mesh: Option<Handle<Mesh>>,
}

/// How close and how far the orbit camera may sit, in hull lengths.
///
/// Both ends are angular and neither mentions a size, which is the point: the zoom is stored
/// as a multiple of the hull's own length, so a player who changes ships keeps the framing
/// they had rather than finding themselves inside a bigger one.
///
/// The far end holds the hull at [`MIN_HULL_PX`] across. The near end stops it exactly filling
/// the window — any closer and the ends are off screen, which is not a view of a ship.
pub fn boom_limits(rad_per_px: f32, fov_x_rad: f32) -> (f64, f64) {
    // How far back a hull has to be to subtend `angle`, in its own lengths. A body of radius
    // `a` at distance `d` covers `2 asin(a/d)`, and taking the ovoid's long semi-axis as that
    // radius is the case where it is broadside and largest.
    //
    // The sine matters at the near stop and nowhere else. There the camera is less than a
    // length away and `a/d` is several per cent short of the angle it stands for — enough to
    // hang the nose and the tail off the edges of the window, which is what the first version
    // of this did.
    let boom_for = |angle: f64| 0.5 / (angle * 0.5).sin().max(f64::MIN_POSITIVE);
    let far = if rad_per_px > 0.0 {
        boom_for(MIN_HULL_PX * rad_per_px as f64)
    } else {
        f64::from(u16::MAX)
    };
    let near = if fov_x_rad > 0.0 { boom_for(fov_x_rad as f64) } else { 1.0 };
    (near, far.max(near))
}

/// The limits the boom is held to for the view on screen, or `None` for leaving it alone.
///
/// **A thumbnail is not a viewfinder.** In the map's mode the world's camera draws into a
/// square 190 points wide, and both limits are angular: clamped against that, going to the map
/// would pull the player's own framing in, and coming back would not give it away again.
/// `None` on the first frames too, where there is no viewport measured yet.
fn held_to(view: crate::ui::ViewMode, size: Option<Vec2>, fov_y: f32) -> Option<(f64, f64)> {
    if view != crate::ui::ViewMode::World {
        return None;
    }
    let size = size?;
    let rad_per_px = crate::starfield::radians_per_pixel(fov_y, size.y);
    Some(boom_limits(rad_per_px, fov_x(fov_y, size.x / size.y.max(1.0))))
}

/// The horizontal field of view, radians, for a projection given its vertical one.
pub fn fov_x(fov_y: f32, aspect: f32) -> f32 {
    2.0 * ((fov_y * 0.5).tan() * aspect.max(f32::MIN_POSITIVE)).atan()
}

/// The rotation putting a hull's nose along `fore` and its belly toward `to_star`, in render axes.
///
/// The mesh's own axes are beam, height and length on `x`, `y` and `z`, so this is the basis
/// `(right, up, fore)` written as a rotation. **`up` is the height axis, which is the hull's
/// collecting face**: a ship rolls it toward its star, which costs nothing — roll about the nose
/// changes no thrust — and is what makes an idle ship visibly broadside. See
/// `lightcone/docs/20-solar-power.md`.
///
/// With no star, or one along the nose where the roll toward it is undetermined, it falls back to
/// ecliptic north; a ship flying straight up the pole then has no preferred roll and any answer is
/// as good as another.
pub fn attitude(fore_sim: DVec3, to_star: Option<DVec3>) -> Quat {
    let fore = fore_sim.normalize_or_zero();
    if fore == DVec3::ZERO {
        return Quat::IDENTITY;
    }
    let toward = to_star
        .map(|s| s.normalize_or_zero() - fore * s.normalize_or_zero().dot(fore))
        .filter(|across| across.length_squared() > 1.0e-6);
    if let Some(up) = toward.map(|across| across.normalize()) {
        let right = up.cross(fore);
        return Quat::from_mat3(&Mat3::from_cols(
            sim_to_render(right).as_vec3(),
            sim_to_render(up).as_vec3(),
            sim_to_render(fore).as_vec3(),
        ));
    }
    let reference = if fore.z.abs() > 0.999 { DVec3::X } else { DVec3::Z };
    let up = (reference - fore * reference.dot(fore)).normalize_or_zero();
    // `right x up = fore`, so the basis is right-handed and survives the change of axes, which
    // is a proper rotation rather than a mirror.
    let right = up.cross(fore);
    let columns = Mat3::from_cols(
        sim_to_render(right).as_vec3(),
        sim_to_render(up).as_vec3(),
        sim_to_render(fore).as_vec3(),
    );
    Quat::from_mat3(&columns)
}

/// The mesh scale for a hull of `length_m`, in render units.
pub fn half_extents(length_m: f64) -> Vec3 {
    let half = length_m * 0.5 / UNIT_M;
    Vec3::new(
        (half * BEAM_PER_LENGTH) as f32,
        (half * HEIGHT_PER_LENGTH) as f32,
        half as f32,
    )
}

/// Put the eye a boom's length behind the ship, along the way it is looking.
///
/// Ahead of everything in [`crate::app::Stage::Scene`]: the eye is what the rest of the frame
/// is drawn relative to, and one drawn against last frame's would shear the scene against the
/// ship every time the view turned.
pub fn place_eye(
    mut ui: ResMut<crate::app::Ui>,
    game: Res<crate::app::Game>,
    uplink: Res<Uplink>,
    camera: Query<(&Projection, &Camera), With<crate::app::SkyCamera>>,
    mut eye: ResMut<Eye>,
) {
    let measured = match camera.single() {
        Ok((Projection::Perspective(perspective), camera)) => {
            held_to(ui.view, camera.logical_viewport_size(), perspective.fov)
        }
        _ => None,
    };
    // Whatever the interface has, left alone rather than clamped against a view nobody is
    // being shown.
    let (near, far) = measured.unwrap_or((ui.boom_lengths, ui.boom_lengths));
    let (anchored, at_ly, length_m) = anchor(&ui, &game, &uplink);
    ui.boom_lengths = ui.boom_lengths.clamp(near, far);
    let boom_m = ui.boom_lengths * length_m;
    eye.boom_m = boom_m;
    eye.anchored = anchored;
    eye.at_ly = at_ly - ui.look.forward() * (boom_m / M_PER_LY);
}

/// Which craft the camera is behind, where it is and how long it is.
///
/// Falls back to the player's own for a perspective naming a craft that is not in sight — a
/// contact that has gone is one the camera cannot follow, and hanging the view on its last
/// known position would be a picture of somewhere nothing is.
fn anchor(
    ui: &crate::ui::UiState,
    game: &Session,
    uplink: &Uplink,
) -> (Option<ShipId>, DVec3, f64) {
    let own = (None, game.ship.motion.position_ly, game.ship.length_m);
    let Some(crate::ui::CameraPerspective::Pov(ship_id)) = ui.perspective else { return own };
    if game.ship.id.0 == ship_id.0 {
        return own;
    }
    match uplink.contacts.iter().find(|c| c.ship_id == ship_id) {
        Some(contact) => (Some(ship_id), contact.position_ly, contact.length_m),
        None => own,
    }
}

/// What a lit hull sends the eye, as linear display light before the tone map.
///
/// Through the band mapping and not the tone map, which the shader now evaluates itself — so a
/// hull and the planet behind it are exposed by one curve rather than two that agree.
fn shading(session: &Session, star_radius_m: f64, star_teff_k: f64, star_distance_m: f64) -> Vec3 {
    let radiance =
        crate::resolved::lit_radiance(ALBEDO, star_radius_m, star_teff_k, star_distance_m);
    Vec3::from_array(session.mapping.apply(&radiance))
}

/// Where the light on a hull comes from, and how bright it is there.
///
/// The system's own star where there is one. Between the stars there is no system to ask, so
/// it is the nearest star in the catalogue — which at that range contributes almost nothing,
/// and the point of it is that a hull out there is a silhouette with a direction rather than a
/// uniformly unlit blob.
pub fn lighting(session: &Session) -> Option<(DVec3, f64, f64)> {
    if let Some(system) = session.system.as_ref() {
        return Some((system.star_position_ly(), system.star_radius_m(), system.star_teff_k()));
    }
    let here = session.ship.motion.position_ly;
    let nearest = session
        .stars
        .iter()
        .min_by(|a, b| here.distance_squared(a.position_ly).total_cmp(&here.distance_squared(b.position_ly)))?;
    Some((nearest.position_ly, nearest.star.radius_m, nearest.star.teff_k))
}

/// What a hull radiates on its own account, as linear display light.
///
/// A blackbody at [`HULL_K`], and nothing else about the craft enters it: a surface at `T` has
/// radiance `B(T)` whichever way it is turned and however far from a star it is. So this is the
/// term that makes a ship visible between the stars, and the term that makes one impossible to
/// hide in the thermal bands.
fn emitted(session: &Session) -> Vec3 {
    Vec3::from_array(session.mapping.apply(&hull_radiance()))
}

/// The same, before the band mapping. Split out because the exposure meters against radiance
/// and the shader wants display light.
///
/// Computed once. It is a function of [`HULL_K`] alone, and each band is a 32-interval Simpson
/// over the Planck curve — a couple of hundred `exp` calls that used to be paid again for every
/// hull in the scene, twice a frame.
fn hull_radiance() -> PerBand<f32> {
    static RADIANCE: std::sync::LazyLock<PerBand<f32>> = std::sync::LazyLock::new(|| {
        PerBand::new(std::array::from_fn(|i| {
            blackbody::band_radiance(em_spectra::Band::ALL[i], HULL_K) as f32
        }))
    });
    *RADIANCE
}

fn uniforms(
    to_star: DVec3,
    reflected: Vec3,
    emitted: Vec3,
    tone: &crate::tonemap::ToneMap,
) -> BodySurfaceUniform {
    BodySurfaceUniform {
        dark: GRAY,
        light: GRAY,
        to_star: sim_to_render(to_star.normalize_or_zero()).as_vec3().extend(NIGHT),
        // A contrast of zero is what turns the generated surface off: the shader mixes the
        // palette at a half whatever the noise says, and the two ends are the same gray.
        params: Vec4::new(0.0, 0.0, 0.0, 0.0),
        reflected: reflected.extend(0.0),
        // `w` is how far the pattern inverts in the body's own light, and a hull has no
        // pattern: its two palette ends are the same gray.
        emitted: emitted.extend(0.0),
        exposure: Vec4::new(tone.surface_reference, tone.surface_stops, 0.0, 0.0),
        ..default()
    }
}

/// Everything with a hull this frame: the player's ship, then everybody in sight.
///
/// `facing` may be zero, meaning nothing decided it; resolving that against what was last seen
/// is [`update_hulls`]'s job, because it is the thing that remembers.
fn drawn(game: &Session, uplink: &Uplink, eye: &Eye, look: DVec3) -> Vec<(Option<ShipId>, Placed)> {
    let now = game.coordinate_time_s();
    let mut out = Vec::with_capacity(uplink.contacts.len() + 1);
    // Exactly the boom the eye was pulled back by, for whichever craft the boom is on; a
    // light-year difference for everything else. See [`Eye::anchored`].
    let offset_of = |at_ly: DVec3, ship_id: Option<ShipId>| {
        if ship_id.map(|s| s.0) == eye.anchored.map(|s| s.0) {
            look * eye.boom_m
        } else {
            (at_ly - eye.at_ly) * M_PER_LY
        }
    };
    out.push((
        None,
        Placed {
            offset_m: offset_of(game.ship.motion.position_ly, None),
            length_m: game.ship.length_m,
            // Always somewhere: a hull has an orientation whether or not anything is
            // deciding it, and the world is what remembers which.
            facing: game.ship.facing_at(now).unwrap_or(DVec3::X),
            at_ly: game.ship.motion.position_ly,
        },
    ));
    for contact in &uplink.contacts {
        out.push((
            Some(contact.ship_id),
            Placed {
                offset_m: offset_of(contact.position_ly, Some(contact.ship_id)),
                length_m: contact.length_m,
                facing: contact.facing,
                at_ly: contact.position_ly,
            },
        ));
    }
    out
}

/// One hull, reduced to what the transform and the material need.
struct Placed {
    /// From the eye, in simulation axes, meters.
    offset_m: DVec3,
    length_m: f64,
    /// Unit, or zero where nothing decides it.
    facing: DVec3,
    /// Where it is in the world, for working out how lit it is.
    at_ly: DVec3,
}

/// Keep a mesh for every craft in sight, and the player's own.
///
/// A hull lives exactly as long as its craft is in the set, and is placed on the frame it is
/// spawned: respawning them all whenever a contact came or went blinked every hull, the
/// player's own included, for a frame.
pub fn update_hulls(
    mut commands: Commands,
    game: Res<crate::app::Game>,
    ui: Res<crate::app::Ui>,
    uplink: Res<Uplink>,
    eye: Res<Eye>,
    mut hulls: ResMut<Hulls>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodySurfaceMaterial>>,
    surfaces: Res<crate::surfaces::Surfaces>,
    mut placed: Query<(Entity, &mut Transform, &MeshMaterial3d<BodySurfaceMaterial>, &Hull)>,
) {
    let look = ui.look.forward();
    let want = drawn(&game.0, &uplink, &eye, look);
    let star = lighting(&game.0);
    // One mapping, so one answer: nothing about a particular hull enters its own heat.
    let own = emitted(&game.0);
    let shade = |at: &Placed| match star {
        Some((star_ly, radius, teff)) => {
            let distance = star_ly.distance(at.at_ly) * M_PER_LY;
            uniforms(star_ly - at.at_ly, shading(&game.0, radius, teff, distance), own, &game.0.tone)
        }
        // No star to reflect. The hull still glows with its own heat, which is the whole
        // reason a ship between the stars is a thing you can see at all.
        None => uniforms(DVec3::Z, Vec3::ZERO, own, &game.0.tone),
    };
    let place = |at: &Placed| Transform {
        translation: sim_to_render(at.offset_m / UNIT_M).as_vec3(),
        rotation: attitude(at.facing, star.map(|(star_ly, _, _)| star_ly - at.at_ly)),
        scale: half_extents(at.length_m),
    };

    let mut kept = Vec::with_capacity(want.len());
    for (entity, mut transform, material, marker) in placed.iter_mut() {
        let Some((id, at)) = want.iter().find(|(id, _)| *id == marker.0) else {
            commands.entity(entity).despawn();
            continue;
        };
        kept.push(*id);
        *transform = place(at);
        let Some(mut asset) = materials.get_mut(&material.0) else { continue };
        let next = shade(at);
        if asset.uniforms != next {
            asset.uniforms = next;
        }
    }

    for (id, at) in want.iter().filter(|(id, _)| !kept.contains(id)) {
        let mesh = hulls
            .mesh
            .get_or_insert_with(|| meshes.add(Sphere::new(1.0).mesh().uv(LONGITUDES, LATITUDES)))
            .clone();
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(materials.add(surfaces.flat.material(shade(at)))),
            place(at),
            // The same reason a resolved body carries it: these are placed by hand at a
            // scale where a mesh's own bounds say nothing useful about where it lands.
            NoFrustumCulling,
            Hull(*id),
        ));
    }
}

/// What a hull at `at_ly` sends the eye, per band, for metering.
///
/// Both halves, because the exposure has to account for both: a ship is reflected starlight in
/// the optical and its own heat in the infrared, and which one dominates is a question about
/// the band mapping rather than about the ship.
///
/// `star` is [`lighting`]'s answer, passed in rather than asked for: between the stars that is
/// a search over the whole catalogue, and a caller metering a scene wants every hull in it lit
/// by the same one anyway.
pub fn radiance_at(star: Option<(DVec3, f64, f64)>, at_ly: DVec3) -> PerBand<f32> {
    let own = hull_radiance();
    let Some((star_ly, radius, teff)) = star else { return own };
    let lit =
        crate::resolved::lit_radiance(ALBEDO, radius, teff, star_ly.distance(at_ly) * M_PER_LY);
    PerBand::new(std::array::from_fn(|i| {
        let band = em_spectra::Band::ALL[i];
        lit[band] + own[band]
    }))
}

/// How much sky a hull of `length_m` covers from `distance_m`, steradians.
///
/// The ovoid taken as a disc of its own long radius, which is what the exposure wants: a
/// bound on the share of the frame it can take rather than its exact silhouette.
pub fn solid_angle_sr(length_m: f64, distance_m: f64) -> f32 {
    if distance_m <= 0.0 {
        return 0.0;
    }
    let radius = length_m * 0.5;
    (std::f64::consts::PI * (radius / distance_m).powi(2)) as f32
}


#[cfg(test)]
mod tests {
    use super::*;

    const RAD_PER_PX: f32 = 7.67e-4;

    /// **A thumbnail is not a viewfinder.** Both boom limits are angular, so the corner square
    /// the world is drawn in while the map is up would pull the framing in — and leaving the
    /// map would not give it back.
    #[test]
    fn the_map_does_not_reframe_the_world() {
        use crate::ui::ViewMode;
        let wide = Vec2::new(1280.0, 720.0);
        let square = Vec2::new(190.0, 190.0);
        let fov = std::f32::consts::FRAC_PI_4;
        assert!(held_to(ViewMode::Map, Some(square), fov).is_none(), "the map clamped the boom");
        assert!(held_to(ViewMode::World, None, fov).is_none(), "nothing measured yet");
        let (_, wide_far) = held_to(ViewMode::World, Some(wide), fov).expect("a view to hold to");
        let (_, square_far) =
            held_to(ViewMode::World, Some(square), fov).expect("a view to hold to");
        assert!(square_far < wide_far, "the corner is the tighter frame, which is the hazard");
    }

    /// The two ends of the zoom, stated as what they are for: five pixels of hull at one end
    /// and a hull the width of the window at the other.
    #[test]
    fn the_zoom_stops_where_the_hull_stops_being_a_shape() {
        let fov_x = fov_x(std::f32::consts::FRAC_PI_4, 16.0 / 9.0);
        let (near, far) = boom_limits(RAD_PER_PX, fov_x);
        assert!(near < far, "{near} to {far}");

        // The angle a hull covers from `booms` of its own lengths away, stated as the
        // textbook angular diameter rather than by inverting the function under test.
        let subtends = |booms: f64| 2.0 * (0.5 / booms).asin();

        // At the far stop it is five pixels across, whatever size it is — the stops are pure
        // numbers, so one check covers the whole designed range of hulls.
        let px = subtends(far) / RAD_PER_PX as f64;
        assert!((px - MIN_HULL_PX).abs() < 1e-6, "the far stop came out {px} px");
        // And at the near stop, exactly the width of the window.
        let across = subtends(near);
        assert!((across - fov_x as f64).abs() < 1e-9, "the near stop spans {across} of {fov_x}");
    }

    /// The default framing has to be inside the stops, or a ship is clamped the moment it is
    /// drawn and the default means nothing.
    #[test]
    fn the_default_framing_is_a_framing_and_not_a_stop() {
        let (near, far) = boom_limits(RAD_PER_PX, fov_x(std::f32::consts::FRAC_PI_4, 16.0 / 9.0));
        assert!(near < DEFAULT_BOOM_LENGTHS && DEFAULT_BOOM_LENGTHS < far);
    }

    /// A viewport nobody has measured must not collapse the range to a point.
    #[test]
    fn a_camera_with_no_viewport_still_gives_a_usable_range() {
        let (near, far) = boom_limits(0.0, 0.0);
        assert!(near <= far && near > 0.0 && far.is_finite());
    }

    fn render(v: DVec3) -> Vec3 {
        sim_to_render(v).as_vec3()
    }

    /// The nose goes where the ship is pointing, and the hull does not turn inside out on the
    /// way through the change of axes.
    #[test]
    fn the_nose_points_along_the_facing_in_render_axes() {
        for fore in [DVec3::X, DVec3::Y, -DVec3::X, DVec3::new(1.0, 2.0, -0.5).normalize()] {
            let q = attitude(fore, None);
            let nose = q * Vec3::Z;
            assert!((nose - render(fore)).length() < 1e-5, "{fore} gave {nose}");
            // A rotation and not a reflection: the three axes stay right-handed.
            let (x, y, z) = (q * Vec3::X, q * Vec3::Y, q * Vec3::Z);
            assert!((x.cross(y) - z).length() < 1e-5, "handedness was lost on {fore}");
        }
    }

    /// Straight up the pole, where the roll is genuinely undefined. Any answer will do; a
    /// degenerate one will not.
    #[test]
    fn a_ship_flying_up_the_pole_still_gets_a_rotation() {
        let q = attitude(DVec3::Z, None);
        assert!(q.is_normalized(), "{q:?}");
        assert!((q * Vec3::Z - render(DVec3::Z)).length() < 1e-5);
    }

    #[test]
    fn nothing_deciding_the_attitude_is_not_a_broken_rotation() {
        assert_eq!(attitude(DVec3::ZERO, None), Quat::IDENTITY);
        assert_eq!(attitude(DVec3::ZERO, Some(DVec3::X)), Quat::IDENTITY);
    }

    /// **The collecting face is the one turned to the star.** A hull broadside in the ecliptic is
    /// the case the old ecliptic-north roll could never draw: its star is in the plane, so north
    /// is across the beam rather than through the belly.
    #[test]
    fn a_hull_rolls_its_belly_toward_its_star() {
        let fore = DVec3::Y;
        let to_star = DVec3::X * 3.0;
        let q = attitude(fore, Some(to_star));
        // `y` is the height axis, which the mesh is thinnest along: it points at the star.
        let up = q * Vec3::Y;
        assert!((up - render(DVec3::X)).length() < 1e-5, "{up}");
        assert!((q * Vec3::Z - render(fore)).length() < 1e-5);
        let (x, y, z) = (q * Vec3::X, q * Vec3::Y, q * Vec3::Z);
        assert!((x.cross(y) - z).length() < 1e-5, "handedness was lost");

        // A star along the nose leaves the roll undetermined, and falls back rather than breaking.
        let along = attitude(DVec3::X, Some(DVec3::X * 2.0));
        assert!(along.is_normalized());
        assert!((along * Vec3::Z - render(DVec3::X)).length() < 1e-5);
    }

    /// Five by three by one, at whatever size, in the renderer's own units.
    #[test]
    fn the_hull_keeps_its_proportions_at_every_size() {
        for length in [500.0, 5_000.0, 50_000.0] {
            let e = half_extents(length);
            // Relative, because the numbers are billionths: a render unit is an astronomical
            // unit and a hull is meters, so `f32` carries seven digits of a very small one.
            let want = length * 0.5 / UNIT_M;
            assert!((e.z as f64 - want).abs() < want * 1e-6, "{length} m gave {}", e.z);
            assert!((e.x / e.z - 0.6).abs() < 1e-5, "beam {e:?}");
            assert!((e.y / e.z - 0.2).abs() < 1e-5, "height {e:?}");
        }
    }

    /// The smallest hull at the closest the camera may come still clears the near plane, or
    /// the ship is clipped away at exactly the zoom a player reaches for to look at it.
    #[test]
    fn the_nearest_the_camera_comes_is_still_outside_the_near_plane() {
        let (near, _) = boom_limits(RAD_PER_PX, fov_x(std::f32::consts::FRAC_PI_4, 16.0 / 9.0));
        let length = lc_world::craft::LENGTH_RANGE_M.0;
        // Boom to the center, less the half-length the nose reaches back toward the camera.
        let clearance = (near - 0.5) * length / UNIT_M;
        assert!(clearance > crate::app::NEAR_PLANE as f64, "{clearance} units of clearance");
    }
}
