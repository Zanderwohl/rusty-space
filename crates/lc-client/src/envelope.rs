//! Population envelopes: the shells that stand in for swarms, belts and clouds.
//!
//! A population has no members, so nothing is instanced from it and nothing invents members to
//! instance. The shell *is* the distribution: its radius is the population's orbital radius and
//! its opacity at each latitude is the population's own sky density there. A belt's inclinations
//! are narrow so the density is a band near the plane and it reads as a ring; an isotropic
//! swarm's is flat and it reads as a sphere. One shape, no special cases.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use em_render::population_material::{
    ATTRIBUTE_SHELL_DENSITY, PopulationMaterial, PopulationUniform,
};
use em_render::render_space::sim_to_render;
use glam::DVec3;
use lc_world::population::Population;

use crate::system::{M_PER_LY, UNIT_M};

/// Latitude and longitude divisions of a shell. Enough that a narrow belt is not a polygon.
pub const RINGS: usize = 48;
pub const SEGMENTS: usize = 96;

/// Overall gain on the mapped opacity.
///
/// Above one because the compressed values are still small: a belt lands at 0.0013 and would be
/// a single dim pixel of a ring. This is the display decision on top of the physical one.
pub const OPACITY_GAIN: f32 = 8.0;

/// What a shell is dimmed to when the ship is inside it.
///
/// Low, because "inside" spans a lot of ground. At one astronomical unit the Kuiper shell is
/// forty times further out than the ship and reads as an even wash; at Uranus it is barely
/// twice as far, lines of sight through it are oblique, and at a fade that suited the first
/// case it became a grey barrel filling the frame.
pub const INSIDE_FADE: f32 = 0.09;

/// Below this covering fraction a population is not drawn at all.
///
/// An Oort cloud covers about 4e-15 of its star's sky. It is photometrically invisible, which is
/// the fact 03-world-model.md builds the shell radius on, and drawing it would say the opposite
/// of what the model says.
pub const FAINTEST: f64 = 1.0e-13;

/// Covering fraction as an opacity.
///
/// A fourth root, and the exponent is the whole decision. The quantity spans fourteen decades —
/// an asteroid belt covers 2.6e-12 of its star's sky, a Kuiper analogue 3e-8, a half-built swarm
/// 0.4 — so a linear mapping renders everything natural as exactly zero.
///
/// A logarithm was the first attempt and overcorrected badly: it put the Kuiper belt at 0.46,
/// and since the ship is *inside* that shell the result was a grey haze over the entire sky. The
/// compression has to leave the ordering intact without flattening it. A fourth root gives a
/// belt 0.001, a Kuiper belt 0.013 and a half-built swarm 0.80: a trace, a haze, and a
/// structure, which is the right reading of all three.
///
/// An envelope is a visualisation either way. It is an orbit line, not a photograph.
pub fn opacity_of(covering: f64) -> f32 {
    if covering <= FAINTEST {
        return 0.0;
    }
    (covering.powf(0.25) as f32).clamp(0.0, 1.0)
}

#[derive(Component)]
pub struct EnvelopeMesh;

/// A ring system, which unlike a population is attached to a body and therefore moves.
#[derive(Component)]
pub struct RingMesh {
    /// The body's name, to find it again in this frame's drawables.
    pub body: String,
    /// Outer radius in render units.
    pub radius: f32,
}

/// One population, as something to draw.
pub struct Shell {
    pub mesh: Handle<Mesh>,
    pub material: Handle<PopulationMaterial>,
    /// Radius in render units, and the rotation taking `+Z` to the population's pole.
    pub radius: f32,
    pub orientation: Quat,
}

/// A unit sphere carrying each vertex's sky density, normalised so the peak is one.
///
/// Built from the population's inclination distribution alone, so it is static: what changes as
/// the ship moves is the transform, never the mesh.
pub fn build_shell(population: &Population) -> Mesh {
    let mut positions = Vec::with_capacity((RINGS + 1) * (SEGMENTS + 1));
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut density = Vec::with_capacity(positions.capacity());
    let mut indices = Vec::with_capacity(RINGS * SEGMENTS * 6);

    // Peak density, to normalise against. In the plane for anything with a plane.
    let peak = (0..=RINGS)
        .map(|r| {
            let phi = latitude(r);
            population.inclination.sky_density(phi)
        })
        .fold(0.0f64, f64::max)
        .max(f64::MIN_POSITIVE);

    for r in 0..=RINGS {
        let phi = latitude(r);
        let at = (population.inclination.sky_density(phi) / peak) as f32;
        let (sp, cp) = phi.sin_cos();
        for s in 0..=SEGMENTS {
            let theta = std::f64::consts::TAU * s as f64 / SEGMENTS as f64;
            let (st, ct) = theta.sin_cos();
            // The population's own frame: its pole is +Z, and the transform turns it.
            let dir = DVec3::new(cp * ct, cp * st, sp);
            let p = sim_to_render(dir).as_vec3().to_array();
            positions.push(p);
            // A sphere's normal at a point is the point.
            normals.push(p);
            density.push(at);
        }
    }

    let row = SEGMENTS + 1;
    for r in 0..RINGS {
        for s in 0..SEGMENTS {
            let a = (r * row + s) as u32;
            let (b, c, d) = (a + 1, a + row as u32, a + row as u32 + 1);
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(ATTRIBUTE_SHELL_DENSITY, density);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Radial divisions of a ring. Enough that the Cassini Division is a gap rather than a hint.
pub const RADIAL: usize = 96;

/// A flat annulus carrying each vertex's optical depth, normalised so the deepest band is one.
///
/// A ring is not a shell: it has radial structure and no latitude. Drawing it as a shell at one
/// radius would put Saturn's rings on a circle, and they span a factor of 1.8 in radius with a
/// division in the middle that is the most recognisable thing about them.
///
/// Unit scale is the outer edge, so the transform is one number.
pub fn build_ring(rings: &lc_world::rings::RingSystem) -> Mesh {
    let (inner, outer) = (rings.inner_m(), rings.outer_m());
    let peak = rings.bands.iter().map(|b| b.optical_depth).fold(0.0f64, f64::max);
    let peak = peak.max(f64::MIN_POSITIVE);

    let mut positions = Vec::with_capacity((RADIAL + 1) * (SEGMENTS + 1));
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut density = Vec::with_capacity(positions.capacity());
    let mut indices = Vec::with_capacity(RADIAL * SEGMENTS * 6);

    let pole = sim_to_render(DVec3::Z).as_vec3().to_array();
    for r in 0..=RADIAL {
        let radius_m = inner + (outer - inner) * r as f64 / RADIAL as f64;
        // Sampled at the midpoint of the step, so a band edge does not fall exactly on a
        // vertex and vanish.
        let at = (rings.depth_at(radius_m + (outer - inner) * 0.5 / RADIAL as f64) / peak) as f32;
        let unit = radius_m / outer;
        for s in 0..=SEGMENTS {
            let theta = std::f64::consts::TAU * s as f64 / SEGMENTS as f64;
            let (st, ct) = theta.sin_cos();
            positions.push(sim_to_render(DVec3::new(unit * ct, unit * st, 0.0)).as_vec3().to_array());
            // Flat: every normal is the pole.
            normals.push(pole);
            density.push(at);
        }
    }

    let row = SEGMENTS + 1;
    for r in 0..RADIAL {
        for s in 0..SEGMENTS {
            let a = (r * row + s) as u32;
            let (b, c, d) = (a + 1, a + row as u32, a + row as u32 + 1);
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(ATTRIBUTE_SHELL_DENSITY, density);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

fn latitude(ring: usize) -> f64 {
    -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * ring as f64 / RINGS as f64
}

/// The uniforms for one population: what the photometry says is there.
pub fn uniforms(population: &Population, seed: f32, gain: f32, inside: bool) -> PopulationUniform {
    let covering = population.covering_fraction();
    // Dust reddens where solid bodies do not, and 04-stellar-photometry.md makes that the
    // grey-versus-reddening diagnostic. The tint says which one the player is looking at.
    let response = population.band_response;
    let reddens = response[em_spectra::Band::B] < response[em_spectra::Band::I] * 0.8;
    let tint = if reddens {
        Vec4::new(0.85, 0.60, 0.40, 1.0)
    } else {
        Vec4::new(0.74, 0.75, 0.78, 1.0)
    };
    PopulationUniform {
        tint,
        opacity: (opacity_of(covering) * gain).clamp(0.0, 1.0),
        seed,
        inside_fade: if inside { INSIDE_FADE } else { 1.0 },
        ..default()
    }
}

/// Is this population worth drawing at all?
pub fn visible(population: &Population) -> bool {
    opacity_of(population.covering_fraction()) > 0.0 && population.thermal_radius() > 0.0
}

/// Where a shell sits and how large, given where the ship is.
///
/// The camera never translates, so the shell is placed relative to it. One render unit is one
/// astronomical unit, which keeps a belt's radius a number of order ten.
pub fn transform(star_ly: DVec3, ship_ly: DVec3, population: &Population) -> Transform {
    let offset_m = (star_ly - ship_ly) * M_PER_LY;
    let radius = (population.thermal_radius() / UNIT_M) as f32;
    Transform {
        translation: sim_to_render(offset_m / UNIT_M).as_vec3(),
        rotation: orientation(population.pole),
        scale: Vec3::splat(radius),
    }
}

/// The rotation taking the mesh's `+Z` to the population's pole.
pub fn orientation(pole: DVec3) -> Quat {
    let to = sim_to_render(pole.normalize_or_zero()).as_vec3();
    if to.length_squared() <= 0.0 {
        return Quat::IDENTITY;
    }
    Quat::from_rotation_arc(sim_to_render(DVec3::Z).as_vec3(), to.normalize())
}

/// Spawn one entity per drawable population of the system the ship is in.
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PopulationMaterial>,
    populations: &[Population],
    gain: f32,
) -> Vec<Shell> {
    populations
        .iter()
        .enumerate()
        .filter(|(_, p)| visible(p))
        .map(|(i, p)| {
            let mesh = meshes.add(build_shell(p));
            let material = materials.add(PopulationMaterial {
                uniforms: uniforms(p, i as f32 * 7.31 + 1.0, gain, true),
            });
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                // An Oort shell is a hundred thousand units across and the ship is inside it;
                // its bounds say nothing useful about whether it is on screen.
                NoFrustumCulling,
                EnvelopeMesh,
            ));
            Shell {
                mesh,
                material,
                radius: (p.thermal_radius() / UNIT_M) as f32,
                orientation: orientation(p.pole),
            }
        })
        .collect()
}

/// Spawn a ring for every body in the system that has one.
fn spawn_rings(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PopulationMaterial>,
    drawn: &[crate::system::Drawable],
    gain: f32,
) {
    for (i, body) in drawn.iter().enumerate() {
        let Some(rings) = body.rings else { continue };
        // Rings are solid and reflective rather than a shadow, so their opacity is what they
        // cover of their own annulus, not of a sphere.
        let annulus = std::f64::consts::PI * rings.system.outer_m().powi(2);
        let covering = rings.system.cross_section_m2() / annulus.max(f64::MIN_POSITIVE);
        // No display gain, unlike a population. The gain exists because even a Kuiper belt is
        // a trace and would otherwise be nothing; Saturn's rings cover a third of their own
        // annulus and need no help. Applying it here made Jupiter's rings -- three parts per
        // million, and it took Voyager to find them -- render at a fifth opacity.
        let _ = gain;
        let uniform = PopulationUniform {
            tint: Vec4::new(0.88, 0.84, 0.76, 1.0),
            opacity: opacity_of(covering),
            seed: i as f32 * 3.77 + 0.5,
            // A ring is not a cloud: its texture is banding, not speckle.
            grain_frequency: 12.0,
            grain_strength: 0.35,
            ..default()
        };
        commands.spawn((
            Mesh3d(meshes.add(build_ring(rings.system))),
            MeshMaterial3d(materials.add(PopulationMaterial { uniforms: uniform })),
            Transform::default(),
            NoFrustumCulling,
            RingMesh {
                body: body.name.clone(),
                radius: (rings.system.outer_m() / UNIT_M) as f32,
            },
        ));
    }
}

/// The envelopes currently drawn, and which system they belong to.
#[derive(Resource, Default)]
pub struct Envelopes {
    pub star: Option<lc_world::sky::StarId>,
    pub shells: Vec<Shell>,
}

/// Spawn the envelopes of the system the ship is in, and keep them placed.
pub fn update_envelopes(
    mut commands: Commands,
    session: Res<crate::app::Game>,
    ui: Res<crate::app::Ui>,
    bodies: Res<crate::starfield::Bodies>,
    mut envelopes: ResMut<Envelopes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PopulationMaterial>>,
    existing: Query<Entity, Or<(With<EnvelopeMesh>, With<RingMesh>)>>,
    mut placed: Query<&mut Transform, (With<EnvelopeMesh>, Without<RingMesh>)>,
    mut ringed: Query<(&mut Transform, &RingMesh), Without<EnvelopeMesh>>,
) {
    let here = session.system.as_ref().map(|s| s.star);
    if envelopes.star != here {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        envelopes.star = here;
        envelopes.shells = match session.system.as_ref() {
            Some(system) => {
                spawn_rings(&mut commands, &mut meshes, &mut materials, &bodies.drawn, ui.envelope_gain);
                spawn(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    &system.populations,
                    ui.envelope_gain,
                )
            }
            None => Vec::new(),
        };
        // Nothing is placed until next frame, when the spawns exist.
        return;
    }

    let Some(system) = session.system.as_ref() else { return };
    for (mut transform, shell) in placed.iter_mut().zip(&envelopes.shells) {
        transform.translation =
            sim_to_render((system.origin_ly - session.ship.position_ly) * M_PER_LY / UNIT_M).as_vec3();
        transform.rotation = shell.orientation;
        transform.scale = Vec3::splat(shell.radius);
    }

    // Rings ride their body, so unlike a shell they are placed every frame.
    for (mut transform, ring) in ringed.iter_mut() {
        let Some(body) = bodies.drawn.iter().find(|d| d.name == ring.body) else { continue };
        let Some(rings) = body.rings else { continue };
        transform.translation =
            sim_to_render((body.position_ly - session.ship.position_ly) * M_PER_LY / UNIT_M).as_vec3();
        transform.rotation = orientation(rings.pole);
        transform.scale = Vec3::splat(ring.radius);
    }

    // The display gain is a knob, so it has to reach the material rather than only the spawn.
    for (shell, population) in
        envelopes.shells.iter().zip(system.populations.iter().filter(|p| visible(p)))
    {
        if let Some(material) = materials.get_mut(&shell.material) {
            let inside = session.ship.position_ly.distance(system.origin_ly) * M_PER_LY
                < population.thermal_radius();
            let next = uniforms(population, material.uniforms.seed, ui.envelope_gain, inside);
            if material.uniforms != next {
                material.uniforms = next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use lc_world::distribution::{Distribution, Inclination};

    use super::*;

    fn population(inclination: Inclination, count: f64) -> Population {
        Population {
            pole: DVec3::Z,
            semi_major: Distribution::normal(3.0e11, 2.0e10, 9),
            eccentricity: Distribution::uniform(0.0, 0.05, 5),
            inclination,
            count,
            cross_section: 3.0e6,
            band_response: em_spectra::PerBand::splat(1.0),
            radiating_ratio: Population::SPHERICAL,
        }
    }

    fn normals(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() {
            bevy_mesh::VertexAttributeValues::Float32x3(v) => v.clone(),
            _ => panic!("normals must be Float32x3"),
        }
    }

    fn densities(mesh: &Mesh) -> Vec<f32> {
        match mesh.attribute(ATTRIBUTE_SHELL_DENSITY).unwrap() {
            bevy_mesh::VertexAttributeValues::Float32(v) => v.clone(),
            _ => panic!("density must be Float32"),
        }
    }

    /// One shape for both: a narrow inclination spread makes a band near the plane, which reads
    /// as a ring, and an isotropic one fills the sphere. Nothing special-cases either.
    #[test]
    fn a_belt_is_a_band_and_a_swarm_is_a_shell() {
        let belt = densities(&build_shell(&population(Inclination::uniform_angle(0.0, 0.2, 12), 1e6)));
        let swarm = densities(&build_shell(&population(Inclination::isotropic(), 1e6)));

        let lit = |v: &[f32]| v.iter().filter(|d| **d > 0.05).count() as f32 / v.len() as f32;
        assert!(lit(&belt) < 0.2, "a belt should cover a fraction of the sphere: {}", lit(&belt));
        assert!(lit(&swarm) > 0.8, "a swarm should cover nearly all of it: {}", lit(&swarm));
        assert!(belt.iter().all(|d| (0.0..=1.0).contains(d)), "densities are normalised");
        assert!(belt.iter().any(|d| *d > 0.99), "and the peak is one");
    }

    /// The mapping is a fourth root because the quantity spans fourteen decades. A logarithm
    /// overcorrected -- it put the Kuiper belt at 0.46, and since the ship is inside that shell
    /// the sky became a grey wash.
    #[test]
    fn the_opacity_mapping_keeps_three_populations_apart() {
        let belt = opacity_of(2.6e-12);
        let kuiper = opacity_of(3.0e-8);
        let swarm = opacity_of(0.4);
        assert!(belt < kuiper && kuiper < swarm, "{belt} {kuiper} {swarm}");
        assert!(belt > 0.0 && belt < 0.01, "a belt is a trace: {belt}");
        assert!(kuiper > 0.005 && kuiper < 0.05, "a Kuiper belt is a haze: {kuiper}");
        assert!(swarm > 0.5, "a swarm is a structure: {swarm}");
    }

    #[test]
    fn an_oort_cloud_is_not_drawn() {
        // Four parts in 1e15, and 03-world-model.md builds the shell radius on it being
        // invisible. Drawing it would say the opposite of what the model says.
        assert_eq!(opacity_of(4.4e-15), 0.0);
        assert!(!visible(&population(Inclination::isotropic(), 1.0)));
    }

    #[test]
    fn a_shell_is_placed_where_the_star_is_and_scaled_to_its_orbit() {
        let p = population(Inclination::uniform_angle(0.0, 0.2, 12), 1e6);
        let star = DVec3::new(1.0, 2.0, -0.5);
        let at_star = transform(star, star, &p);
        assert!(at_star.translation.length() < 1e-6, "the ship at the star sees it centred");
        assert!((at_star.scale.x - (p.thermal_radius() / UNIT_M) as f32).abs() < 1e-3);

        let away = transform(star, star + DVec3::X * 1e-4, &p);
        assert!(away.translation.length() > 0.0, "and moving the ship moves the shell");
    }

    #[test]
    fn the_pole_orients_the_shell() {
        let flat = orientation(DVec3::Z);
        let tipped = orientation(DVec3::X);
        assert!(flat.is_finite() && tipped.is_finite());
        assert!(flat.angle_between(tipped) > 1.0, "a different pole is a different orientation");
        assert_eq!(orientation(DVec3::ZERO), Quat::IDENTITY);
    }

    #[test]
    fn a_shell_the_ship_is_inside_is_dimmed() {
        let p = population(Inclination::uniform_angle(0.0, 0.2, 12), 1e9);
        let out = uniforms(&p, 0.0, OPACITY_GAIN, false);
        let inside = uniforms(&p, 0.0, OPACITY_GAIN, true);
        assert_eq!(out.inside_fade, 1.0);
        assert!(inside.inside_fade < 0.5, "inside, a shell covers the whole sky");
        assert_eq!(out.opacity, inside.opacity, "only the fade differs, not the physics");
    }

    #[test]
    fn a_ring_mesh_has_the_gaps_the_data_says_it_has() {
        let saturn = lc_world::rings::for_body("Saturn").unwrap();
        let mesh = build_ring(saturn);
        let d = densities(&mesh);
        assert!(d.iter().any(|v| *v > 0.99), "the B ring is the peak");
        assert!(d.iter().any(|v| *v < 0.1 && *v > 0.0), "and the division is thin, not empty");

        // Walking outward, the profile must rise, fall into the division and rise again.
        let row = SEGMENTS + 1;
        let profile: Vec<f32> = (0..=RADIAL).map(|r| d[r * row]).collect();
        let peak = profile.iter().cloned().fold(0.0f32, f32::max);
        let peak_at = profile.iter().position(|v| *v == peak).unwrap();
        let after = &profile[peak_at..];
        let dip = after.iter().cloned().fold(f32::MAX, f32::min);
        assert!(dip < peak / 5.0, "there should be a division past the B ring: {dip} vs {peak}");
        assert!(after.iter().rev().take(10).any(|v| *v > dip * 3.0), "and an A ring past that");
    }

    #[test]
    fn a_ring_is_flat_and_a_shell_is_not() {
        let saturn = lc_world::rings::for_body("Saturn").unwrap();
        let ring = normals(&build_ring(saturn));
        let shell = normals(&build_shell(&population(Inclination::isotropic(), 1e6)));
        let first = ring[0];
        assert!(ring.iter().all(|n| *n == first), "every ring normal is the pole");
        assert!(shell.iter().any(|n| *n != shell[0]), "a shell's normals point everywhere");
    }

    /// The four ring systems, in the order a person would rank them by eye.
    #[test]
    fn the_ring_systems_come_out_in_the_right_order() {
        let drawn = |id: &str| {
            let r = lc_world::rings::for_body(id).unwrap();
            opacity_of(r.cross_section_m2() / (std::f64::consts::PI * r.outer_m().powi(2)))
        };
        let (saturn, uranus, neptune, jupiter) =
            (drawn("Saturn"), drawn("Uranus"), drawn("Neptune"), drawn("Jupiter"));
        assert!(saturn > 0.5, "Saturn's rings are the thing you see: {saturn}");
        assert!(uranus < saturn * 0.5 && uranus > 0.05, "Uranus's are faint but real: {uranus}");
        assert!(neptune < uranus, "Neptune's fainter still: {neptune}");
        assert!(jupiter < 0.05, "and Jupiter's took Voyager to find: {jupiter}");
    }
}
