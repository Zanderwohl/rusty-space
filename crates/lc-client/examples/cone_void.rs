//! `em_render::exhaust_cone_material` photographed in a void: a stand-in hull burning at 5 g, its
//! aperture, and its exhaust cone out to the courtesy radius, and nothing else.
//!
//! ```text
//! cargo run -p lc-client --example cone_void -- --view beside --shot /tmp/cone.png
//! cargo run -p lc-client --example cone_void -- --length 50000 --view inside \
//!     --drift 0.05 --shot /tmp/gsv.png --frames 60 --burst 4
//! ```
//!
//! | flag | for |
//! |---|---|
//! | `--length <m>` | the hull, scaled from the starting ship as 31's tables are; 500 by default, 50 000 for a GSV |
//! | `--view beside\|behind\|inside` | where the camera stands: off the cone's middle, on its axis past the end, or in it halfway looking at the ship |
//! | `--drift <deg>` | how far the camera turns about what it looks at each frame, so a burst is a moving camera |
//! | `--aperture <m>` | the drive's open face, diameter; the hull's height by default |
//! | `--shot <path> --frames <n> --burst <n>` | as the client: `n` consecutive frames after `--frames` |
//!
//! Every position is worked out in `f64` relative to the eye, and the camera sits at the render
//! origin, as the client places everything.

use bevy::camera::{Hdr, RenderTarget};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use em_render::exhaust_cone_material::{
    ApertureGlowMaterial, ApertureGlowUniform, ExhaustConeMaterial, ExhaustConeMaterialPlugin,
    ExhaustConeUniform,
};
use em_spectra::{Band, BandMapping, PerBand, blackbody, presets};
use glam::{DQuat, DVec3};
use lc_client::ui::HAZARD;
use lc_world::courtesy::{cooking_flux_w_m2, drive_courtesy_radius_m};
use lc_world::craft::{BEAM_PER_LENGTH, HEIGHT_PER_LENGTH};
use lc_world::emit::{aperture_temperature_k, rating_w};
use lc_world::fitting::Balance;

/// How bright the cone is where it would cook, and at the courtesy radius, in the hazard color.
const HOT_GAIN: f32 = 0.35;
const FAINT_GAIN: f32 = 0.02;

/// How far the near-field glow runs aft of the face and how wide it is, in aperture radii.
const GLOW_REACH: f32 = 6.0;
const GLOW_WIDTH: f32 = 1.0;
/// How far over the exposure's reference the glow sits side-on through its middle. A display
/// choice, as `plume::CORE_STOPS` is: the face is ten decades over and carries the physics.
const GLOW_STOPS: f64 = 4.0;
/// See `PlumeUniform::exposure`.
const OVERFLOW_GAIN: f32 = 0.5;
const STOPS: f32 = 5.0;

const SUN_RADIUS_M: f64 = 6.957e8;
const SUN_K: f64 = 5772.0;
const AU_M: f64 = 1.495_978_707e11;

/// Where the exhaust goes, in render axes.
const AFT: DVec3 = DVec3::X;
const WINDOW: (u32, u32) = (1280, 720);

#[derive(Resource, Clone)]
struct Args {
    length_m: f64,
    view: String,
    drift_deg: f64,
    aperture_m: Option<f64>,
    shot: Option<String>,
    frames: u32,
    burst: u32,
}

impl Args {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let value = |flag: &str| {
            args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
        };
        let number = |flag: &str| {
            value(flag).map(|v| v.parse::<f64>().unwrap_or_else(|_| panic!("{flag} {v}")))
        };
        let view = value("--view").unwrap_or_else(|| "beside".into());
        assert!(
            ["beside", "behind", "inside"].contains(&view.as_str()),
            "--view beside|behind|inside, not {view}",
        );
        Self {
            length_m: number("--length").unwrap_or(500.0),
            view,
            drift_deg: number("--drift").unwrap_or(0.0),
            aperture_m: number("--aperture"),
            shot: value("--shot"),
            frames: number("--frames").unwrap_or(30.0) as u32,
            burst: number("--burst").unwrap_or(1.0) as u32,
        }
    }
}

/// The burn, from `lc_world` and nothing typed here.
#[derive(Resource)]
struct Burn {
    power_w: f64,
    half_angle_rad: f64,
    courtesy_m: f64,
    hot_w_m2: f64,
    aperture_m: f64,
    face_k: f64,
}

impl Burn {
    /// 31's tables: the starting ship's drive section, scaled with the hull as length cubed.
    fn of(args: &Args) -> Self {
        let b = Balance::DEFAULT;
        let drive_m3 = 5.0 * lc_world::form::presets::SLOT_M3;
        let power_w = rating_w(&b, drive_m3 * (args.length_m / 500.0).powi(3));
        let aperture_m = args.aperture_m.unwrap_or(args.length_m * HEIGHT_PER_LENGTH);
        let face_m2 = std::f64::consts::PI * (0.5 * aperture_m).powi(2);
        Self {
            power_w,
            half_angle_rad: b.drive_spread_rad,
            courtesy_m: drive_courtesy_radius_m(&b, power_w),
            hot_w_m2: cooking_flux_w_m2(&b),
            aperture_m,
            face_k: aperture_temperature_k(power_w, face_m2),
        }
    }
}

#[derive(Resource, Default)]
struct Frame(u32);

#[derive(Component)]
struct Cone;

#[derive(Component)]
struct Glow;

#[derive(Component)]
struct HullPart;

fn main() {
    let args = Args::parse();
    let burn = Burn::of(&args);
    info!(
        "{} m hull: {:.2e} W, courtesy radius {:.3e} m, aperture {} m at {:.3e} K",
        args.length_m, burn.power_w, burn.courtesy_m, burn.aperture_m, burn.face_k,
    );
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Lightcone Frontier — exhaust cone in a void".into(),
                resolution: WINDOW.into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(ExhaustConeMaterialPlugin)
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(args)
        .insert_resource(burn)
        .init_resource::<Frame>()
        .add_systems(Startup, stage)
        .add_systems(First, |mut frame: ResMut<Frame>| frame.0 += 1)
        .add_systems(Update, (place, photograph))
        .run();
}

fn mapped(mapping: &BandMapping, radiance: &PerBand<f64>) -> DVec3 {
    DVec3::from_array(std::array::from_fn(|c| {
        Band::ALL.iter().map(|b| mapping.matrix[c][b.index()] as f64 * radiance[*b]).sum()
    }))
}

fn luminance(rgb: DVec3) -> f64 {
    rgb.dot(DVec3::new(0.2126, 0.7152, 0.0722))
}

fn planck(kelvin: f64) -> PerBand<f64> {
    PerBand::new(std::array::from_fn(|i| blackbody::band_radiance(Band::ALL[i], kelvin)))
}

/// The eye and what it looks at, from the apex, meters, `frame` frames in.
fn camera(args: &Args, burn: &Burn, frame: u32) -> (DVec3, DVec3) {
    let l = burn.courtesy_m;
    let side = DVec3::Z;
    let (eye, target) = match args.view.as_str() {
        "beside" => (AFT * 0.5 * l + side * 0.8 * l + DVec3::Y * 0.12 * l, AFT * 0.5 * l),
        // Past the far end and just off the axis: every ray runs nearly along it.
        "behind" => (AFT * 1.6 * l + DVec3::Y * 0.02 * l, DVec3::ZERO),
        // Halfway, a third of the way out from the axis to the wall.
        _ => {
            let wall = 0.5 * l * burn.half_angle_rad.tan();
            (AFT * 0.5 * l + DVec3::Y * wall / 3.0, DVec3::ZERO)
        }
    };
    // Across the axis: turning about it leaves a round cone exactly as it was.
    let turn = DQuat::from_axis_angle(DVec3::Z, (args.drift_deg * frame as f64).to_radians());
    (target + turn * (eye - target), target)
}

fn stage(
    mut commands: Commands,
    burn: Res<Burn>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut cones: ResMut<Assets<ExhaustConeMaterial>>,
    mut glows: ResMut<Assets<ApertureGlowMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let mapping = presets::natural();
    let sunlit = planck(SUN_K).map(|_, v| v * lc_client::hull::ALBEDO * (SUN_RADIUS_M / AU_M).powi(2));
    let reference = luminance(mapped(&mapping, &sunlit));
    let face = mapped(&mapping, &planck(burn.face_k));
    let glow = face * (reference * 2f64.powf(GLOW_STOPS) / luminance(face));
    info!("exposure {reference:.3e}, face {:.3e} ({:.1} stops over)", luminance(face), (luminance(face) / reference).log2());

    let hazard = LinearRgba::from(HAZARD).to_vec3();
    commands.spawn((
        Mesh3d(meshes.add(ExhaustConeMaterial::proxy(burn.half_angle_rad as f32))),
        MeshMaterial3d(cones.add(ExhaustConeMaterial {
            uniforms: ExhaustConeUniform {
                emission: Vec4::new(
                    burn.power_w as f32,
                    burn.half_angle_rad as f32,
                    burn.courtesy_m as f32,
                    burn.hot_w_m2 as f32,
                ),
                faint: (hazard * FAINT_GAIN).extend(0.0),
                hot: (hazard * HOT_GAIN).extend(0.0),
                ..default()
            },
        })),
        Transform::default(),
        NoFrustumCulling,
        Cone,
    ));
    commands.spawn((
        Mesh3d(meshes.add(ApertureGlowMaterial::proxy(GLOW_REACH, GLOW_WIDTH))),
        MeshMaterial3d(glows.add(ApertureGlowMaterial {
            uniforms: ApertureGlowUniform {
                face: face.as_vec3().extend(0.0),
                glow: glow.as_vec3().extend(0.0),
                shape: Vec4::new(GLOW_REACH, GLOW_WIDTH, 0.0, 0.0),
                exposure: Vec4::new(reference as f32, STOPS, OVERFLOW_GAIN, 0.0),
                ..default()
            },
        })),
        Transform::default(),
        NoFrustumCulling,
        Glow,
    ));
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(64, 32))),
        MeshMaterial3d(standard.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.31, 0.33),
            perceptual_roughness: 0.8,
            ..default()
        })),
        Transform::default(),
        NoFrustumCulling,
        HullPart,
    ));
    commands.spawn((
        DirectionalLight { illuminance: 4_000.0, ..default() },
        Transform::from_xyz(-0.4, 1.0, 0.7).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Into an image rather than the window: a window behind others presents nothing, and its
    // screenshot comes back black. `COPY_SRC` is what a screenshot of an image copies out by.
    let mut film = Image::new_target_texture(WINDOW.0, WINDOW.1, TextureFormat::Rgba8UnormSrgb, None);
    film.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let film = images.add(film);
    commands.insert_resource(Film(film.clone()));
    commands.spawn((
        Camera3d::default(),
        RenderTarget::Image(film.into()),
        Projection::Perspective(PerspectiveProjection { far: 1.0e12, ..default() }),
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
        Transform::default(),
    ));
}

#[derive(Resource)]
struct Film(Handle<Image>);

/// Everything placed about the eye at the origin, and each proxy told where the eye is in its
/// own space: the transform undone, in `f64`.
fn place(
    args: Res<Args>,
    burn: Res<Burn>,
    frame: Res<Frame>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mut cone: Query<(&mut Transform, &MeshMaterial3d<ExhaustConeMaterial>), (With<Cone>, Without<Camera3d>)>,
    mut glow: Query<
        (&mut Transform, &MeshMaterial3d<ApertureGlowMaterial>),
        (With<Glow>, Without<Camera3d>, Without<Cone>),
    >,
    mut hull: Query<&mut Transform, (With<HullPart>, Without<Camera3d>, Without<Cone>, Without<Glow>)>,
    mut cones: ResMut<Assets<ExhaustConeMaterial>>,
    mut glows: ResMut<Assets<ApertureGlowMaterial>>,
) {
    let (eye, target) = camera(&args, &burn, frame.0);
    let up = if args.view == "beside" { DVec3::Y } else { DVec3::Z };
    for mut transform in &mut cameras {
        *transform = Transform::default().looking_to((target - eye).as_vec3(), up.as_vec3());
    }

    let along = DQuat::from_rotation_arc(DVec3::Y, AFT);
    let apex = -eye;
    let local = |scale: f64| (along.inverse() * (eye / scale)).as_vec3().extend(0.0);

    if let Ok((mut transform, handle)) = cone.single_mut() {
        *transform = Transform {
            translation: apex.as_vec3(),
            rotation: along.as_quat(),
            scale: Vec3::splat(burn.courtesy_m as f32),
        };
        if let Some(mut material) = cones.get_mut(&handle.0) {
            material.uniforms.eye_local = local(burn.courtesy_m);
        }
    }
    let radius = 0.5 * burn.aperture_m;
    if let Ok((mut transform, handle)) = glow.single_mut() {
        *transform = Transform {
            translation: apex.as_vec3(),
            rotation: along.as_quat(),
            scale: Vec3::splat(radius as f32),
        };
        if let Some(mut material) = glows.get_mut(&handle.0) {
            material.uniforms.eye_local = local(radius);
        }
    }
    if let Ok(mut transform) = hull.single_mut() {
        let half = 0.5 * args.length_m;
        *transform = Transform {
            translation: (apex - AFT * half).as_vec3(),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(
                half as f32,
                (half * HEIGHT_PER_LENGTH) as f32,
                (half * BEAM_PER_LENGTH) as f32,
            ),
        };
    }
}

/// `lc_client::dev::photograph`, from the film: consecutive frames of one run, numbered, then quit.
fn photograph(
    mut commands: Commands,
    args: Res<Args>,
    frame: Res<Frame>,
    film: Res<Film>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &args.shot else { return };
    let burst = args.burst.max(1);
    let first = args.frames;
    if (first..first + burst).contains(&frame.0) {
        let index = frame.0 - first;
        let at = match (burst > 1, path.rsplit_once('.')) {
            (false, _) => path.clone(),
            (true, Some((stem, extension))) => format!("{stem}.{index}.{extension}"),
            (true, None) => format!("{path}.{index}"),
        };
        commands.spawn(Screenshot::image(film.0.clone())).observe(save_to_disk(at));
    }
    // The capture is asynchronous; quitting on the same frame loses the file.
    if frame.0 > first + burst + 30 {
        exit.write(AppExit::Success);
    }
}
