//! A field held at a temperature around a stand-in hull, in a void, for photographing
//! `shaders/field.wgsl`.
//!
//! The binary's own `--field-k` holds the *player's* field and arrives with R11 of
//! `lightcone/docs/plans/forms-and-fields.md`; there is no player field before then.
//!
//! ```text
//! cargo run -p lc-client --example field_void -- --field-k 2400 --mode clear \
//!     --shot /tmp/field.png --frames 60 --burst 4
//! ```
//!
//! | flag | for |
//! |---|---|
//! | `--field-k <kelvin>` | the field's temperature; 400 by default |
//! | `--mode clear\|black` | the field's mode |
//! | `--fill <fraction>` | heat over the limit; by default `(T / LIMIT_K)⁴`, since heat goes as `T⁴` |
//! | `--spot <strength>` | one beam's hot spot, from the upper left, as a multiple of the field's own power |
//! | `--switch <progress>` | a switch into `--mode` from the other, frozen at this progress |
//! | `--collapse <seconds>` | the field collapsed this long ago; `--afterglow <s>` sets how long the afterglow runs |
//! | `--au <distance>` | from a Sun-like star; 1 by default |
//! | `--mapping <name>` | one of `em_spectra::presets::all()` |
//! | `--yaw <deg>` | turn the ship |
//! | `--shot <path> --frames <n> --burst <n>` | photograph `n` consecutive frames after `--frames`, as the client does |

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::asset::RenderAssetUsages;
use em_render::body_surface_material::{BodySurfaceMaterial, BodySurfaceMaterialPlugin, BodySurfaceUniform};
use em_render::field_material::{
    BLACK, CLEAR, FieldMaterial, FieldMaterialPlugin, FieldUniform, RAMP, ramp_entry, ramp_kelvin,
};
use em_spectra::{Band, BandMapping, PerBand, blackbody, presets};

/// The field's limit, where it collapses: 30-the-field.md's anchors.
const LIMIT_K: f64 = 4600.0;
/// 30-the-field.md's `clear_absorptivity`.
const CLEAR_ABSORPTIVITY: f32 = 0.3;
/// 30-the-field.md's `collapse_spike_k`.
const SPIKE_K: f32 = 1.0e7;

const SUN_RADIUS_M: f64 = 6.957e8;
const SUN_K: f64 = 5772.0;
const AU_M: f64 = 1.495_978_707e11;

const HULL_M: f32 = 100.0;
/// How far the envelope stands off the hull on every side, meters.
const STANDOFF_M: f32 = 22.0;
/// Where the Mind sits, from the hull's center toward the bow, in hull lengths.
const MIND_FORE: f32 = 0.2;
const CAMERA_M: f32 = 210.0;
/// How many times its own size the debris of a collapse spreads to.
const DEBRIS_REACH: f32 = 4.0;
/// From the ship toward the camera.
const CAMERA_DIR: Vec3 = Vec3::new(0.62, 0.30, 0.72);
/// A gray world straight behind the ship, so a Black field has something to hide against.
const BACKDROP_M: f32 = 900.0;
const BACKDROP_BEHIND_M: f32 = 6000.0;
/// See `PlumeUniform::exposure`; a field fills more of the frame than a plume, and blooms less.
const OVERFLOW_GAIN: f32 = 0.5;
/// `lc_client::app`'s window size, so a shot here compares with a shot there.
const WINDOW: (u32, u32) = (1280, 720);

#[derive(Resource, Clone)]
struct Args {
    kelvin: f64,
    black: bool,
    fill: Option<f64>,
    spot: f32,
    switch: Option<f32>,
    collapse: Option<f32>,
    afterglow: f32,
    au: f64,
    mapping: BandMapping,
    yaw: f32,
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
        let number = |flag: &str| value(flag).map(|v| v.parse::<f64>().unwrap_or_else(|_| panic!("{flag} {v}")));
        let mapping = value("--mapping").map_or_else(presets::natural, |name| {
            presets::all()
                .into_iter()
                .find(|(n, _)| *n == name)
                .unwrap_or_else(|| panic!("no mapping {name}"))
                .1
        });
        let black = match value("--mode").as_deref() {
            None | Some("clear") => false,
            Some("black") => true,
            Some(other) => panic!("--mode clear|black, not {other}"),
        };
        Self {
            kelvin: number("--field-k").unwrap_or(400.0),
            black,
            fill: number("--fill"),
            spot: number("--spot").unwrap_or(0.0) as f32,
            switch: number("--switch").map(|p| p as f32),
            collapse: number("--collapse").map(|s| s as f32),
            afterglow: number("--afterglow").unwrap_or(30.0) as f32,
            au: number("--au").unwrap_or(1.0),
            mapping,
            yaw: number("--yaw").unwrap_or(0.0) as f32,
            shot: value("--shot"),
            frames: number("--frames").unwrap_or(60.0) as u32,
            burst: number("--burst").unwrap_or(1.0) as u32,
        }
    }

    fn fill(&self) -> f32 {
        self.fill.unwrap_or((self.kelvin / LIMIT_K).powi(4)) as f32
    }
}

fn main() {
    let args = Args::parse();
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Lightcone Frontier — field in a void".into(),
                resolution: WINDOW.into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((BodySurfaceMaterialPlugin, FieldMaterialPlugin))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(args)
        .add_systems(Startup, stage)
        .add_systems(Update, (shade, photograph))
        .run();
}

#[derive(Component)]
struct Field;

#[derive(Component)]
struct Hull;

/// The scene's exposure and light, worked out once: nothing in it moves.
#[derive(Resource)]
struct Lighting {
    to_star: Vec3,
    /// A white Lambertian surface facing the star, display light.
    starlight: Vec3,
    reference: f32,
    stops: f32,
    spectrum: [Vec4; RAMP],
}

fn mapped(mapping: &BandMapping, radiance: &PerBand<f64>) -> [f64; 3] {
    // In `f64`: a visible band at 400 K is under `f32`'s smallest number.
    std::array::from_fn(|c| {
        Band::ALL.iter().map(|b| mapping.matrix[c][b.index()] as f64 * radiance[*b]).sum()
    })
}

fn luminance(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn planck(kelvin: f64) -> PerBand<f64> {
    PerBand::new(std::array::from_fn(|i| blackbody::band_radiance(Band::ALL[i], kelvin)))
}

/// Sunlight on a surface of `albedo` facing it, per band.
fn sunlit(albedo: f64, au: f64) -> PerBand<f64> {
    let dilution = albedo * (SUN_RADIUS_M / (au * AU_M)).powi(2);
    planck(SUN_K).map(|_, v| v * dilution)
}

/// `Session::expose_to_percentile(0.98)`, over the three things in the void.
fn meter(samples: &mut [(f64, f64)]) -> f32 {
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = samples.iter().map(|s| s.1).sum();
    let mut below = 0.0;
    for (brightness, weight) in samples.iter() {
        below += weight;
        if below >= 0.98 * total {
            return *brightness as f32;
        }
    }
    samples.last().map_or(1.0, |s| s.0 as f32)
}

/// Off to one side of the ship, so the envelope's edge crosses its limb.
fn backdrop_at() -> Vec3 {
    let toward = CAMERA_DIR.normalize();
    let right = Vec3::Y.cross(toward).normalize();
    -toward * BACKDROP_BEHIND_M + right * 1900.0 - Vec3::Y * 300.0
}

fn hull_extents() -> Vec3 {
    let half = HULL_M * 0.5;
    Vec3::new(
        half * lc_world::craft::BEAM_PER_LENGTH as f32,
        half * lc_world::craft::HEIGHT_PER_LENGTH as f32,
        half,
    )
}

fn envelope_extents() -> Vec3 {
    hull_extents() + Vec3::splat(STANDOFF_M)
}

fn stage(
    mut commands: Commands,
    args: Res<Args>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut surfaces_materials: ResMut<Assets<BodySurfaceMaterial>>,
    mut fields: ResMut<Assets<FieldMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let to_star = Vec3::new(-0.55, 0.45, 0.70).normalize();
    let hull_radiance = sunlit(lc_client::hull::ALBEDO, args.au);
    let own = planck(lc_world::craft::HULL_K);
    let white = mapped(&args.mapping, &sunlit(1.0, args.au));

    let absorbs = if args.black { 1.0 } else { CLEAR_ABSORPTIVITY as f64 };
    let field_rgb = mapped(&args.mapping, &planck(args.kelvin)).map(|c| c * absorbs);
    let lit_rgb = mapped(&args.mapping, &hull_radiance);
    let own_rgb = mapped(&args.mapping, &own);
    let hull_lum = luminance(std::array::from_fn(|c| lit_rgb[c] + own_rgb[c]));

    let sr = |radius: f32, distance: f32| std::f64::consts::PI * (radius as f64 / distance as f64).powi(2);
    let envelope = envelope_extents();
    let mut samples = [
        (hull_lum, sr(hull_extents().z * 0.6, CAMERA_M)),
        (luminance(field_rgb), sr(envelope.z * 0.7, CAMERA_M)),
        (hull_lum, sr(BACKDROP_M, BACKDROP_BEHIND_M)),
    ];
    let reference = meter(&mut samples);
    let stops = 5.0;

    let spectrum = std::array::from_fn(|i| ramp_entry(mapped(&args.mapping, &planck(ramp_kelvin(i) as f64))));
    let lighting = Lighting {
        to_star,
        starlight: Vec3::from_array(white.map(|c| c as f32)),
        reference,
        stops,
        spectrum,
    };
    info!(
        "field {} K, {} mode, fill {:.3}; exposure {:.3e}, field {:.3e}, hull {:.3e}",
        args.kelvin,
        if args.black { "black" } else { "clear" },
        args.fill(),
        reference,
        luminance(field_rgb),
        hull_lum,
    );

    let surface = |lit: [f64; 3]| BodySurfaceUniform {
        dark: Vec4::new(0.30, 0.31, 0.33, 1.0),
        light: Vec4::new(0.30, 0.31, 0.33, 1.0),
        to_star: to_star.extend(0.10),
        params: Vec4::ZERO,
        reflected: Vec3::from_array(lit.map(|c| c as f32)).extend(0.0),
        emitted: Vec3::from_array(own_rgb.map(|c| c as f32)).extend(0.0),
        exposure: Vec4::new(reference, stops, 0.0, 0.0),
        ..default()
    };
    let turn = Quat::from_rotation_y(args.yaw.to_radians());
    let sphere = meshes.add(Sphere::new(1.0).mesh().uv(64, 32));
    commands.spawn((
        Mesh3d(sphere.clone()),
        MeshMaterial3d(surfaces_materials.add(flat(surface(lit_rgb), &mut images))),
        Transform { rotation: turn, scale: hull_extents(), ..default() },
        Hull,
    ));
    commands.spawn((
        Mesh3d(sphere),
        MeshMaterial3d(surfaces_materials.add(flat(surface(lit_rgb), &mut images))),
        Transform::from_translation(backdrop_at()).with_scale(Vec3::splat(BACKDROP_M)),
    ));

    // The envelope is meshed in its own units so the collapse can round it about its center;
    // a scale in the transform would stretch the sphere back into an ellipsoid.
    let envelope_mesh = meshes.add(Sphere::new(1.0).mesh().uv(96, 48).scaled_by(envelope));
    let uniforms = uniforms(&args, &lighting, 0.0);
    for layer in FieldMaterial::layers(uniforms, envelope.max_element()) {
        commands.spawn((
            Mesh3d(envelope_mesh.clone()),
            MeshMaterial3d(fields.add(layer)),
            Transform::from_rotation(turn),
            Field,
        ));
    }
    commands.insert_resource(lighting);

    commands.spawn((
        Camera3d::default(),
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
        // Stood back for a collapse, which grows past where the camera otherwise is.
        Transform::from_translation(CAMERA_DIR.normalize() * CAMERA_M * if args.collapse.is_some() { 5.0 } else { 1.0 })
            .looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// The surface material with every texture a uniform gray cube: the palette's two ends are the
/// same gray and the relief's slope is zero, so none of them shows.
fn flat(uniforms: BodySurfaceUniform, images: &mut Assets<Image>) -> BodySurfaceMaterial {
    let mut cube = Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 6 },
        TextureDimension::D2,
        &[128, 128, 128, 255],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    cube.texture_view_descriptor =
        Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    let cube = images.add(cube);
    BodySurfaceMaterial {
        uniforms,
        pattern: cube.clone(),
        color: cube.clone(),
        weather_0: cube.clone(),
        weather_1: cube.clone(),
        weather_2: cube.clone(),
        climate: cube.clone(),
        land: cube.clone(),
        ice: cube.clone(),
        growth: cube.clone(),
        sand: cube.clone(),
        height: cube,
    }
}

fn uniforms(args: &Args, lighting: &Lighting, clock_s: f32) -> FieldUniform {
    let envelope = envelope_extents();
    let origin = Vec3::new(0.0, 0.0, HULL_M * MIND_FORE);
    let reach = envelope.z + origin.z;
    let (mode, previous, progress) = match (args.black, args.switch) {
        (black, Some(p)) => {
            let new = if black { BLACK } else { CLEAR };
            (new, BLACK - new, p)
        }
        (true, None) => (BLACK, BLACK, 1.0),
        (false, None) => (CLEAR, CLEAR, 1.0),
    };
    let mut hot_spots = [Vec4::ZERO; em_render::field_material::HOT_SPOTS];
    if args.spot > 0.0 {
        hot_spots[0] = Vec3::new(0.2, 0.5, 1.0).normalize().extend(args.spot);
    }
    FieldUniform {
        state: Vec4::new(args.kelvin as f32, args.fill(), CLEAR_ABSORPTIVITY, clock_s),
        mode: Vec4::new(mode, previous, progress, reach),
        origin: origin.extend(0.0),
        to_star: lighting.to_star.extend(0.0),
        starlight: lighting.starlight.extend(0.0),
        exposure: Vec4::new(lighting.reference, lighting.stops, OVERFLOW_GAIN, 0.0),
        hot_spots,
        collapse: Vec4::new(args.collapse.unwrap_or(-1.0), 0.5, args.afterglow, DEBRIS_REACH),
        collapse_k: Vec4::new(SPIKE_K, LIMIT_K as f32, envelope.max_element(), 0.0),
        spectrum: lighting.spectrum,
    }
}

fn shade(
    time: Res<Time>,
    args: Res<Args>,
    lighting: Option<Res<Lighting>>,
    fields: Query<&MeshMaterial3d<FieldMaterial>, With<Field>>,
    mut hulls: Query<&mut Visibility, With<Hull>>,
    mut materials: ResMut<Assets<FieldMaterial>>,
) {
    let Some(lighting) = lighting else { return };
    let mut next = uniforms(&args, &lighting, time.elapsed_secs());
    if let Some(since) = args.collapse {
        // The game's meter follows the scene, so the afterglow is exposed for as it cools; the
        // flash is left to overflow. The cooling is the shader's own.
        let cooled = (since / args.afterglow).clamp(0.0, 1.0) as f64;
        let kelvin = LIMIT_K * (1.0 - cooled).powf(0.6) + 300.0;
        next.exposure.x = luminance(mapped(&args.mapping, &planck(kelvin))) as f32;
    }
    for handle in &fields {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.uniforms = next.clone();
        }
    }
    if args.collapse.is_some() {
        for mut visibility in &mut hulls {
            *visibility = Visibility::Hidden;
        }
    }
}

/// `lc_client::dev::photograph`: consecutive frames of one run, numbered, then quit.
fn photograph(
    mut commands: Commands,
    args: Res<Args>,
    mut frames: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &args.shot else { return };
    *frames += 1;
    let burst = args.burst.max(1);
    if (args.frames..args.frames + burst).contains(&*frames) {
        let index = *frames - args.frames;
        let at = match (burst > 1, path.rsplit_once('.')) {
            (false, _) => path.clone(),
            (true, Some((stem, extension))) => format!("{stem}.{index}.{extension}"),
            (true, None) => format!("{path}.{index}"),
        };
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(at));
    }
    if *frames > args.frames + burst + 30 {
        exit.write(AppExit::Success);
    }
}
