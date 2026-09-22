//! Which graph draws a resolved body's surface, and the cubemap it bakes into.
//!
//! `textures/surfaces.lcsurfaces` decides: a body named under `[bodies]` gets that graph, and
//! every other body its class's. So a hand-made Earth is a file and a line, not a change here.
//! The graph supplies only the pattern; palette and contrast stay the class's, from
//! `lc_world::surface`, and each body's seed is its name's.

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

#[derive(Asset, TypePath, Debug, Deserialize)]
pub struct SurfaceManifest {
    /// Paths under `textures/`.
    pub classes: HashMap<Surface, String>,
    #[serde(default)]
    pub bodies: HashMap<String, String>,
}

impl SurfaceManifest {
    pub fn graph_for(&self, name: &str, class: Surface) -> Option<&str> {
        self.bodies
            .get(name)
            .or_else(|| self.classes.get(&class))
            .map(String::as_str)
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

#[derive(Resource)]
pub struct Surfaces {
    manifest: Handle<SurfaceManifest>,
    /// For what is drawn with the surface material but has no pattern of its own: hulls.
    pub flat: Handle<Image>,
    /// Kept for the session. A body's pattern never changes, and bodies stop and start being
    /// resolved as the ship moves; re-baking each time would be most of the cost.
    by_body: HashMap<String, Handle<Image>>,
    /// Waiting on the manifest to say which graph.
    unrouted: Vec<(String, Surface, Handle<Image>)>,
}

impl FromWorld for Surfaces {
    fn from_world(world: &mut World) -> Self {
        let manifest = world.resource::<AssetServer>().load(MANIFEST);
        let flat = world
            .resource_mut::<Assets<Image>>()
            .add(placeholder(Target::new(Shape::Cube(1))));
        Self {
            manifest,
            flat,
            by_body: HashMap::new(),
            unrouted: Vec::new(),
        }
    }
}

impl Surfaces {
    /// The pattern `name` is drawn with: flat at first, its own once baked.
    pub fn pattern(
        &mut self,
        name: &str,
        class: Surface,
        images: &mut Assets<Image>,
    ) -> Handle<Image> {
        if let Some(image) = self.by_body.get(name) {
            return image.clone();
        }
        let image = images.add(placeholder(Target::new(Shape::Cube(FACE))));
        self.by_body.insert(name.to_owned(), image.clone());
        self.unrouted.push((name.to_owned(), class, image.clone()));
        image
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
    if surfaces.unrouted.is_empty() {
        return;
    }
    let Some(manifest) = manifests.get(&surfaces.manifest) else {
        if let LoadState::Failed(e) = assets.load_state(&surfaces.manifest) {
            warn!("the surface manifest did not load, drawing bodies flat: {e}");
            surfaces.unrouted.clear();
        }
        return;
    };
    for (name, class, image) in std::mem::take(&mut surfaces.unrouted) {
        match manifest.graph_for(&name, class) {
            Some(path) => {
                let graph = assets.load(format!("textures/{path}"));
                bakes.request(graph, seed_of(&name), Target::new(Shape::Cube(FACE)), image);
            }
            None => warn!("no surface graph for {name} or its class, {class:?}"),
        }
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
    use texture_graph_core::{EvalCtx, Graph, LayerKind, cube_sample, eval, load_from_str};

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

    /// Every class reaches a graph, and every graph is one a sphere bake accepts. A kind the bake
    /// refuses would fail only at run time, leaving that class drawn flat.
    #[test]
    fn every_class_has_a_graph_a_sphere_can_bake() {
        let manifest = manifest();
        for class in ALL {
            let path = manifest
                .graph_for("nobody in particular", class)
                .expect("routed");
            for layer in &graph(path).layers {
                assert!(
                    matches!(
                        layer.kind,
                        LayerKind::Color(_)
                            | LayerKind::Noise(_)
                            | LayerKind::Coordinate(_)
                            | LayerKind::Mix(_)
                            | LayerKind::MinMax(_)
                            | LayerKind::Wave(_)
                    ),
                    "{path} uses {}, which a sphere bake refuses",
                    layer.kind.category_label(),
                );
            }
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
        let mut manifest = manifest();
        manifest
            .bodies
            .insert("Earth".into(), "bodies/earth.tgraph".into());
        assert_eq!(
            manifest.graph_for("Earth", Surface::Weathered),
            Some("bodies/earth.tgraph")
        );
        assert_eq!(
            manifest.graph_for("Mars", Surface::Weathered),
            manifest
                .classes
                .get(&Surface::Weathered)
                .map(String::as_str)
        );
    }
}
