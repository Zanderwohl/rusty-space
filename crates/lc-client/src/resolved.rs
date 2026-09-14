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

use crate::session::Session;
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
    // It does clip, often. The exposure is placed by the star field and a lit surface a few
    // astronomical units from its star is tens of stops above that, so most resolved bodies
    // come out at the top of the window and their palette carries the difference. Making the
    // exposure account for what is actually on screen is the fix, and it is its own piece of
    // work: today the auto-exposure looks only at stars.
    session.tone.shade(&radiance, &session.mapping).value
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
    if star_distance_m <= 0.0 {
        return PerBand::splat(0.0);
    }
    let scale = body.surface.albedo() * (star_radius_m / star_distance_m).powi(2);
    PerBand::new(std::array::from_fn(|i| {
        (blackbody::band_radiance(Band::ALL[i], star_teff_k) * scale) as f32
    }))
}

fn uniforms(session: &Session, body: &Drawable, star_ly: DVec3, level: f32) -> BodySurfaceUniform {
    let (dark, light, contrast) = body.surface.palette();
    let to_star = sim_to_render((star_ly - body.position_ly).normalize_or_zero()).as_vec3();
    // Stable per body and independent of everything else, so a world looks the same every time
    // it is approached.
    let seed = (crate::system::name_seed(&body.name) % 100_000) as f32 * 1.0e-2;
    BodySurfaceUniform {
        dark: Vec4::new(dark[0], dark[1], dark[2], 1.0),
        light: Vec4::new(light[0], light[1], light[2], 1.0),
        to_star: to_star.extend(NIGHT),
        params: Vec4::new(level, contrast, seed, if body.surface.is_banded() { 1.0 } else { 0.0 }),
    }
}

/// Keep a sphere for every body close enough to be one.
pub fn update_resolved(
    mut commands: Commands,
    session: Res<crate::app::Game>,
    bodies: Res<crate::starfield::Bodies>,
    mut resolved: ResMut<Resolved>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodySurfaceMaterial>>,
    camera: Query<(&Projection, &Camera), With<Camera3d>>,
    existing: Query<(Entity, &ResolvedBody)>,
    mut placed: Query<(&mut Transform, &MeshMaterial3d<BodySurfaceMaterial>, &ResolvedBody)>,
) {
    let rad_per_px = crate::starfield::camera_scale(&camera);
    let Some(system) = bodies.system.as_ref() else {
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
        .filter(|d| is_resolved(d, session.position_ly, rad_per_px))
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
            let level = surface_level(&session.0, body, star_radius, star_teff, star_distance);
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(materials.add(BodySurfaceMaterial {
                    uniforms: uniforms(&session.0, body, star_ly, level),
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
            sim_to_render((body.position_ly - session.position_ly) * M_PER_LY / UNIT_M).as_vec3();
        transform.rotation =
            Quat::from_rotation_arc(Vec3::Y, sim_to_render(body.pole).as_vec3().normalize());
        transform.scale = Vec3::splat((body.radius_m / UNIT_M) as f32);

        if let Some(asset) = materials.get_mut(&material.0) {
            let star_distance = star_ly.distance(body.position_ly) * M_PER_LY;
            let level = surface_level(&session.0, body, star_radius, star_teff, star_distance);
            let next = uniforms(&session.0, body, star_ly, level);
            if asset.uniforms != next {
                asset.uniforms = next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use lc_world::sky::{AuthoredStars, StarProvider};
    use lc_world::surface::Surface;

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

    #[test]
    fn a_lit_surface_dims_as_the_inverse_square_of_its_distance_from_the_star() {
        let b = body(6.0e7, DVec3::ZERO);
        let near = v(surface_radiance(&b, 6.957e8, 5772.0, AU));
        let far = v(surface_radiance(&b, 6.957e8, 5772.0, 30.0 * AU));
        assert!((near / far - 900.0).abs() / 900.0 < 1e-6, "{near} against {far}");
        assert_eq!(v(surface_radiance(&b, 6.957e8, 5772.0, 0.0)), 0.0);
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

    /// The displayed window is two and a half stops wide and a lit surface's range is tens, so
    /// the level clips. That is what an exposure placed by the star field does to a planet, and
    /// the palette is what carries the difference until the exposure learns to see bodies.
    #[test]
    fn the_level_is_a_window_and_a_close_surface_fills_it() {
        let mut session = Session::new(&AuthoredStars::sample(), 3);
        session.tone.reference = 2.0;
        let b = body(6.0e7, DVec3::ZERO);
        let near = surface_level(&session, &b, 6.957e8, 5772.0, AU);
        let far = surface_level(&session, &b, 6.957e8, 5772.0, 30.0 * AU);
        assert_eq!(near, 1.0, "a close surface is at the top of the window");
        assert_eq!(far, 0.0, "and one ten stops down is under it");
        assert_eq!(surface_level(&session, &b, 6.957e8, 5772.0, 0.0), 0.0);
    }
}
