//! `em_render::drone_material` photographed in a void: fixture docks and targets on a stand-in
//! hull, and nothing else.
//!
//! ```text
//! cargo run -p lc-client --example drones_void -- --shot /tmp/drones.png --burst 4
//! cargo run -p lc-client --example drones_void -- --shot /tmp/still.png --at 20 --rate 0
//! ```
//!
//! | flag | for |
//! |---|---|
//! | `--shot <path>` | photograph and quit; `--burst <n>` photographs `n` consecutive frames |
//! | `--frames <n>` | frames before the first photograph (default 30) |
//! | `--at <s>` / `--rate <r>` | the clock: `t = at + r * frame / 60`, so `--rate 0` holds it at `at` |
//! | `--mode <working\|dismantle\|idle>` | what the drones are doing |
//! | `--count <n>` / `--seed <n>` | how many drones, and which |
//! | `--distance <m>` | camera distance from the hull, for the haze |
//!
//! The clock counts frames rather than wall time, so a burst is evenly spaced whatever the
//! machine does and two runs at one `--at` agree.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use em_render::drone_material::{DroneMaterial, DroneMaterialPlugin, DroneUniform, drone_quads};

const HULL_RADII: Vec3 = Vec3::new(40.0, 12.0, 12.0);

#[derive(Resource, Clone)]
struct Options {
    shot: Option<String>,
    burst: u32,
    frames: u32,
    at: f32,
    rate: f32,
    mode: String,
    count: u32,
    seed: u32,
    distance: f32,
}

impl Options {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let value = |flag: &str| {
            args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
        };
        fn parsed<T: std::str::FromStr>(flag: &str, v: Option<String>, default: T) -> T {
            v.map(|v| v.parse().unwrap_or_else(|_| panic!("{flag} {v}"))).unwrap_or(default)
        }
        let number = |flag: &str, default: f32| parsed(flag, value(flag), default);
        let integer = |flag: &str, default: u32| parsed(flag, value(flag), default);
        Self {
            shot: value("--shot"),
            burst: integer("--burst", 1),
            frames: integer("--frames", 30),
            at: number("--at", 0.0),
            rate: number("--rate", 1.0),
            mode: value("--mode").unwrap_or_else(|| "working".into()),
            count: integer("--count", 600),
            seed: integer("--seed", 1),
            distance: number("--distance", 150.0),
        }
    }

    fn clock(&self, frame: u32) -> f32 {
        self.at + self.rate * frame as f32 / 60.0
    }
}

#[derive(Resource, Default)]
struct Frame(u32);

#[derive(Resource)]
struct Drones(Handle<DroneMaterial>);

fn main() {
    let options = Options::parse();
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Drones, in a void".into(),
                resolution: (1280, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(DroneMaterialPlugin)
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(options)
        .init_resource::<Frame>()
        .add_systems(Startup, setup)
        .add_systems(First, |mut frame: ResMut<Frame>| frame.0 += 1)
        .add_systems(Update, (tick, photograph))
        .run();
}

/// Docks in a row across the stern; targets over a band of the bow, where a part is going up.
fn fixtures() -> (Vec<Vec3>, Vec<Vec3>) {
    let docks = (0..6)
        .map(|i| {
            let angle = std::f32::consts::TAU * i as f32 / 6.0;
            Vec3::new(-HULL_RADII.x * 0.98, 5.0 * angle.cos(), 5.0 * angle.sin())
        })
        .collect();
    let targets = (0..48)
        .map(|i| {
            let along = 0.45 + 0.3 * (i % 4) as f32 / 3.0;
            let angle = std::f32::consts::TAU * (i / 4) as f32 / 12.0;
            let x = HULL_RADII.x * along;
            let r = HULL_RADII.y * (1.0 - along * along).sqrt() + 0.8;
            Vec3::new(x, r * angle.cos(), r * angle.sin())
        })
        .collect();
    (docks, targets)
}

fn setup(
    mut commands: Commands,
    options: Res<Options>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut drone_materials: ResMut<Assets<DroneMaterial>>,
) {
    let eye = Vec3::new(0.55, 0.45, 0.7).normalize() * options.distance;
    commands.spawn((Camera3d::default(), Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y)));
    commands.spawn((
        DirectionalLight { illuminance: 4_000.0, ..default() },
        Transform::from_xyz(1.0, 2.0, 0.5).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(64, 32))),
        MeshMaterial3d(standard.add(StandardMaterial {
            base_color: Color::srgb(0.18, 0.19, 0.21),
            perceptual_roughness: 0.8,
            ..default()
        })),
        Transform::from_scale(HULL_RADII),
    ));

    let (docks, targets) = fixtures();
    let dock_mesh = meshes.add(Sphere::new(0.8));
    let dock_light = standard.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(0.6, 0.9, 1.2),
        ..default()
    });
    for dock in &docks {
        commands.spawn((Mesh3d(dock_mesh.clone()), MeshMaterial3d(dock_light.clone()), Transform::from_translation(*dock)));
    }

    let (working, carrying) = match options.mode.as_str() {
        "working" => (0.85, 0.0),
        "dismantle" => (0.85, 1.0),
        "idle" => (0.0, 0.0),
        other => panic!("--mode {other}: working, dismantle or idle"),
    };
    let mut uniforms = DroneUniform {
        patrol_center: Vec4::ZERO,
        patrol_radii: HULL_RADII.extend(0.0),
        time: options.clock(0),
        seed: options.seed,
        count: options.count,
        working,
        patrol: 0.08,
        carrying,
        ..default()
    };
    uniforms.set_docks(&docks);
    uniforms.set_targets(&targets);
    let material = drone_materials.add(DroneMaterial { uniforms });
    commands.insert_resource(Drones(material.clone()));
    commands.spawn((
        Mesh3d(meshes.add(drone_quads(options.count))),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        NoFrustumCulling,
    ));
}

fn tick(options: Res<Options>, frame: Res<Frame>, drones: Res<Drones>, mut materials: ResMut<Assets<DroneMaterial>>) {
    let time = options.clock(frame.0);
    if let Some(mut material) = materials.get_mut(&drones.0)
        && material.uniforms.time != time
    {
        material.uniforms.time = time;
    }
}

fn photograph(mut commands: Commands, options: Res<Options>, frame: Res<Frame>, mut exit: MessageWriter<AppExit>) {
    let Some(path) = &options.shot else { return };
    let burst = options.burst.max(1);
    let first = options.frames;
    if (first..first + burst).contains(&frame.0) {
        let index = frame.0 - first;
        let at = match (burst > 1, path.rsplit_once('.')) {
            (false, _) => path.clone(),
            (true, Some((stem, extension))) => format!("{stem}.{index}.{extension}"),
            (true, None) => format!("{path}.{index}"),
        };
        println!("frame {} at t = {:.4} s -> {at}", frame.0, options.clock(frame.0));
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(at));
    }
    // The capture is asynchronous; quitting on the same frame loses the file.
    if frame.0 > first + burst + 30 {
        exit.write(AppExit::Success);
    }
}
