//! Which graphs draw a resolved body's surface, and the cubemaps they bake into.
//!
//! `textures/surfaces.lcsurfaces` decides. A body takes its class's graph, which supplies only a
//! pattern: palette and contrast stay the class's, from `lc_world::surface`. A body named under
//! `[bodies]` takes that graph instead, baked in color, and one named under `[clouds]` has a
//! cloud deck drawn over it. So a hand-made Earth is a file and a line, not a change here. Each
//! body's seed is its name's.

use std::collections::HashMap;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, LoadState};
use bevy::prelude::*;
use lc_world::surface::Surface;
use serde::Deserialize;

use crate::procedural::{Bakes, Shape, Target, placeholder};

const MANIFEST: &str = "textures/surfaces.lcsurfaces";

/// Texels along a cube face's edge: about two thousand around the equator, for a megabyte and
/// a half a body.
pub const FACE: u32 = 512;

/// The same for a color cubemap, at 24 megabytes. A color graph is the whole surface rather
/// than a variation on one, and it is what a ship in low orbit fills the view with.
pub const COLOR_FACE: u32 = 1024;

#[derive(Asset, TypePath, Debug, Deserialize)]
pub struct SurfaceManifest {
    /// Paths under `textures/`.
    pub classes: HashMap<Surface, String>,
    /// Baked in color, in place of the class's pattern and palette.
    #[serde(default)]
    pub bodies: HashMap<String, String>,
    #[serde(default)]
    pub clouds: HashMap<String, String>,
}

/// The graphs a body is drawn from, as paths under `textures/`.
#[derive(Debug, PartialEq, Eq)]
pub struct Look<'a> {
    pub ground: Ground<'a>,
    pub clouds: Option<&'a str>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Ground<'a> {
    Pattern(&'a str),
    Color(&'a str),
}

impl SurfaceManifest {
    pub fn look_for(&self, name: &str, class: Surface) -> Option<Look<'_>> {
        let ground = match self.bodies.get(name) {
            Some(path) => Ground::Color(path),
            None => Ground::Pattern(self.classes.get(&class)?),
        };
        Some(Look {
            ground,
            clouds: self.clouds.get(name).map(String::as_str),
        })
    }
}

#[derive(Debug)]
pub enum ManifestLoadError {
    Io(std::io::Error),
    Utf8(std::string::FromUtf8Error),
    Toml(toml::de::Error),
}

impl std::fmt::Display for ManifestLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Utf8(e) => write!(f, "{e}"),
            Self::Toml(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for ManifestLoadError {}

/// TOML, under an extension of its own because the library's catalogue already has `toml`.
#[derive(Default, TypePath)]
pub struct ManifestLoader;

impl AssetLoader for ManifestLoader {
    type Asset = SurfaceManifest;
    type Settings = ();
    type Error = ManifestLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load: &mut LoadContext<'_>,
    ) -> Result<SurfaceManifest, Self::Error> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(ManifestLoadError::Io)?;
        let text = String::from_utf8(bytes).map_err(ManifestLoadError::Utf8)?;
        toml::from_str(&text).map_err(ManifestLoadError::Toml)
    }

    fn extensions(&self) -> &[&str] {
        &["lcsurfaces"]
    }
}

/// What a body's surface material binds. Each is a placeholder until its bake lands, and stays
/// one if the manifest gives the body no such graph.
#[derive(Clone)]
pub struct BodyImages {
    pub pattern: Handle<Image>,
    pub color: Handle<Image>,
    pub clouds: Handle<Image>,
}

struct Body {
    images: BodyImages,
    class: Surface,
    /// Whether `color` and `clouds` are drawn; `None` until the manifest has said.
    drawn: Option<(bool, bool)>,
}

#[derive(Resource)]
pub struct Surfaces {
    manifest: Handle<SurfaceManifest>,
    /// For what is drawn with the surface material but has no surface of its own: hulls.
    pub flat: BodyImages,
    /// Kept for the session. A body's surface never changes, and bodies stop and start being
    /// resolved as the ship moves; re-baking each time would be most of the cost.
    by_body: HashMap<String, Body>,
}

impl FromWorld for Surfaces {
    fn from_world(world: &mut World) -> Self {
        let manifest = world.resource::<AssetServer>().load(MANIFEST);
        let mut images = world.resource_mut::<Assets<Image>>();
        let cube = Target::new(Shape::Cube(1));
        let flat = BodyImages {
            pattern: images.add(placeholder(cube)),
            color: images.add(placeholder(cube.color())),
            clouds: images.add(placeholder(cube.color())),
        };
        Self {
            manifest,
            flat,
            by_body: HashMap::new(),
        }
    }
}

impl Surfaces {
    /// What `name` is drawn with: flat at first, its own once baked.
    pub fn images(&mut self, name: &str, class: Surface, images: &mut Assets<Image>) -> BodyImages {
        self.by_body
            .entry(name.to_owned())
            .or_insert_with(|| {
                let cube = Target::new(Shape::Cube(1));
                Body {
                    images: BodyImages {
                        pattern: images.add(placeholder(cube)),
                        color: images.add(placeholder(cube.color())),
                        clouds: images.add(placeholder(cube.color())),
                    },
                    class,
                    drawn: None,
                }
            })
            .images
            .clone()
    }

    /// Whether `name`'s color and cloud cubemaps are drawn, as the material's weights for them.
    pub fn drawn(&self, name: &str) -> (f32, f32) {
        let (color, clouds) = self
            .by_body
            .get(name)
            .and_then(|b| b.drawn)
            .unwrap_or_default();
        (f32::from(u8::from(color)), f32::from(u8::from(clouds)))
    }
}

/// Stable per body and independent of everything else, so a world looks the same every time it
/// is approached, and on every client.
fn seed_of(name: &str) -> u32 {
    (crate::system::name_seed(name) >> 32) as u32
}

fn route(
    mut surfaces: ResMut<Surfaces>,
    manifests: Res<Assets<SurfaceManifest>>,
    assets: Res<AssetServer>,
    mut bakes: ResMut<Bakes>,
) {
    if surfaces.by_body.values().all(|b| b.drawn.is_some()) {
        return;
    }
    let Some(manifest) = manifests.get(&surfaces.manifest) else {
        if let LoadState::Failed(e) = assets.load_state(&surfaces.manifest) {
            warn!("the surface manifest did not load, drawing bodies flat: {e}");
            for body in surfaces.by_body.values_mut() {
                body.drawn = Some((false, false));
            }
        }
        return;
    };
    for (name, body) in surfaces.by_body.iter_mut().filter(|(_, b)| b.drawn.is_none()) {
        let Some(look) = manifest.look_for(name, body.class) else {
            warn!("no surface graph for {name} or its class, {:?}", body.class);
            body.drawn = Some((false, false));
            continue;
        };
        let seed = seed_of(name);
        let mut bake = |path: &str, target, image: &Handle<Image>| {
            let graph = assets.load(format!("textures/{path}"));
            bakes.request(graph, seed, target, image.clone());
        };
        let pattern = Target::new(Shape::Cube(FACE));
        let color = Target::new(Shape::Cube(COLOR_FACE)).color();
        match look.ground {
            Ground::Pattern(path) => bake(path, pattern, &body.images.pattern),
            Ground::Color(path) => bake(path, color, &body.images.color),
        }
        if let Some(path) = look.clouds {
            bake(path, color, &body.images.clouds);
        }
        body.drawn = Some((matches!(look.ground, Ground::Color(_)), look.clouds.is_some()));
    }
}

pub struct SurfacesPlugin;

impl Plugin for SurfacesPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<SurfaceManifest>()
            .init_asset_loader::<ManifestLoader>()
            .init_resource::<Surfaces>()
            .add_systems(Update, route);
    }
}

#[cfg(test)]
mod tests {
    use texture_graph_core::{EvalCtx, Graph, cube_sample, eval, load_from_str};
    use texture_graph_gpu::{Baker, DeviceCtx, ScalarFormat};

    use super::*;

    const ALL: [Surface; 6] = [
        Surface::GasGiant,
        Surface::IceGiant,
        Surface::Ice,
        Surface::Rock,
        Surface::Weathered,
        Surface::Scorched,
    ];

    fn manifest() -> SurfaceManifest {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/textures/surfaces.lcsurfaces"
        );
        toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn graph(path: &str) -> Graph {
        let full = format!("{}/assets/textures/{path}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{full}: {e}"));
        load_from_str(&text)
            .unwrap_or_else(|e| panic!("{full}: {e}"))
            .graph
    }

    fn class_graph(class: Surface) -> Graph {
        graph(manifest().classes.get(&class).expect("routed"))
    }

    /// The layer named `name`, at `s`: its L.
    fn at(g: &Graph, name: &str, s: texture_graph_core::Sample) -> f32 {
        let id = g
            .layers
            .iter()
            .find(|l| l.name == name)
            .unwrap_or_else(|| panic!("no {name}"))
            .id;
        eval::evaluate(g, id, s, &EvalCtx::default()).l
    }

    fn points() -> impl Iterator<Item = texture_graph_core::Sample> {
        (0..6).flat_map(|face| {
            (0..9).flat_map(move |i| {
                (0..9).map(move |j| cube_sample(face, i as f32 / 8.0, j as f32 / 8.0))
            })
        })
    }

    /// Every class reaches a graph, and every graph in the manifest bakes on a sphere: a kind the
    /// bake refuses would fail only at run time, leaving that body drawn flat. Skipped without a
    /// GPU.
    #[test]
    fn every_graph_in_the_manifest_bakes_on_a_sphere() {
        let manifest = manifest();
        for class in ALL {
            assert!(manifest.classes.contains_key(&class), "{class:?} is not routed");
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let Ok(ctx) = runtime.block_on(DeviceCtx::request_headless()) else {
            eprintln!("no GPU; the manifest's graphs were not baked");
            return;
        };
        let mut baker = Baker::new(ctx);
        let eval = EvalCtx::default();
        for path in manifest.classes.values() {
            let g = graph(path);
            baker
                .bake_scalar_cube(&g, g.output.color.unwrap(), 8, ScalarFormat::R8Unorm, &eval)
                .unwrap_or_else(|e| panic!("{path}: {e}"));
        }
        for path in manifest.bodies.values().chain(manifest.clouds.values()) {
            baker
                .bake_color_cube(&graph(path), 8, &eval)
                .unwrap_or_else(|e| panic!("{path}: {e}"));
        }
    }

    /// The banded graphs are body_surface.wgsl's bands as they stood, exactly: the same sum of
    /// height and two noise fields through the same sine, sharpened the same way. Only the
    /// noise underneath is new.
    #[test]
    fn the_bands_are_the_shaders_bands() {
        for class in [Surface::GasGiant, Surface::IceGiant] {
            let g = class_graph(class);
            let out = g.output.color.unwrap();
            for s in points() {
                let y = 2.0 * s.v - 1.0;
                let turbulence = at(&g, "turbulence", s) - 0.5;
                let drift = (at(&g, "drift", s) - 0.5) * 0.9;
                let bands = ((y + turbulence * 0.055) * 18.0 + drift).sin();
                let want = (0.5 + 0.62 * bands).clamp(0.0, 1.0);
                let got = eval::evaluate(&g, out, s, &EvalCtx::default())
                    .l
                    .clamp(0.0, 1.0);
                assert!(
                    (got - want).abs() < 2e-4,
                    "{class:?} at {s:?}: {got} against {want}"
                );
            }
        }
    }

    #[test]
    fn the_mottling_is_the_shaders_mottling() {
        for class in [
            Surface::Ice,
            Surface::Rock,
            Surface::Weathered,
            Surface::Scorched,
        ] {
            let g = class_graph(class);
            let out = g.output.color.unwrap();
            for s in points() {
                let want = (at(&g, "broad", s) * 0.75 + at(&g, "fine", s) * 0.25).clamp(0.0, 1.0);
                let got = eval::evaluate(&g, out, s, &EvalCtx::default())
                    .l
                    .clamp(0.0, 1.0);
                assert!(
                    (got - want).abs() < 2e-4,
                    "{class:?} at {s:?}: {got} against {want}"
                );
            }
        }
    }

    #[test]
    fn a_body_named_in_the_manifest_takes_its_own_graph() {
        let manifest: SurfaceManifest = toml::from_str(
            r#"
            [classes]
            Weathered = "surfaces/weathered.tgraph"
            [bodies]
            Earth = "worlds/earthlike.tgraph"
            [clouds]
            Earth = "worlds/earthlike-clouds.tgraph"
            "#,
        )
        .unwrap();
        assert_eq!(
            manifest.look_for("Earth", Surface::Weathered),
            Some(Look {
                ground: Ground::Color("worlds/earthlike.tgraph"),
                clouds: Some("worlds/earthlike-clouds.tgraph"),
            })
        );
        assert_eq!(
            manifest.look_for("Mercury", Surface::Weathered),
            Some(Look {
                ground: Ground::Pattern("surfaces/weathered.tgraph"),
                clouds: None,
            })
        );
        assert_eq!(manifest.look_for("Mercury", Surface::Rock), None);
    }
}
