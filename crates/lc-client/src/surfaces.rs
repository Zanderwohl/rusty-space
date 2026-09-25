//! Which graphs draw a resolved body's surface, and the cubemaps they bake into.
//!
//! `textures/surfaces.lcsurfaces` decides. A body takes its class's graph, which supplies only a
//! pattern: palette and contrast stay the class's, from `lc_world::surface`. A body named under
//! `[bodies]` takes that graph instead, baked in color, and one named under `[clouds]` has a
//! cloud deck drawn over it. So a hand-made Earth is a file and a line, not a change here. Each
//! body's seed is its name's.
//!
//! A rocky world with air -- anything [`lc_world::climate`] has a climate for -- takes the
//! manifest's `[rocky]` graphs instead, with the graph's parameters bound from its climate: one
//! graph for Earth, Mars and every world the generator makes between and beyond them. A giant
//! takes `[giants]`, bound from [`lc_world::giant`] the same way, and a rocky world without air
//! `[airless]`, from [`lc_world::airless`]; its [`RELIEF`] layer is baked too so
//! body_surface.wgsl can light its craters.
//!
//! A cloud deck's [`WEATHER`] is rebaked every [`CLOUD_PERIOD_S`] with a new seed and blended
//! in body_surface.wgsl; its [`CLIMATE`] is baked once. Keyframes follow coordinate time, so
//! every client draws the same weather. See lightcone/docs/07-rendering.md.

use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, LoadState};
use bevy::prelude::*;
use em_render::body_surface_material::{BodySurfaceMaterial, BodySurfaceUniform};
use lc_world::airless::Airless;
use lc_world::climate::Climate;
use lc_world::giant::Giant;
use lc_world::sky::StarId;
use lc_world::surface::Surface;
use serde::Deserialize;
use texture_graph_gpu::ScalarFormat;
use texture_graph_core::ParamValue;
use texture_graph_core::color::oklcha;

use crate::procedural::{Bakes, Params, Shape, Target, TextureGraph, placeholder};

const MANIFEST: &str = "textures/surfaces.lcsurfaces";

/// Texels along a cube face's edge: about two thousand around the equator, for a megabyte and
/// a half a body.
pub const FACE: u32 = 512;

/// The same for a color cubemap, at 24 megabytes. A color graph is the whole surface rather
/// than a variation on one, and it is what a ship in low orbit fills the view with.
pub const COLOR_FACE: u32 = 1024;

/// It stays below one, so a byte holds it.
const WEATHER: Target = Target::new(Shape::Cube(COLOR_FACE)).layer("zonal");

const CLIMATE: Target = Target::new(Shape::Cube(64)).layer("drive term 1");

/// airless.tgraph's relief about zero, where a byte would lose all but the largest craters.
const RELIEF: Target = Target::new(Shape::Cube(COLOR_FACE)).layer("height").format(ScalarFormat::R16Float);

/// The `relief` of airless.tgraph's Craters nodes: heights per unit of its sample space.
pub const RELIEF_SCALE: f32 = 8.0;

/// [`WEATHER`]'s mean over the sphere, measured; it varies about half a percent by seed.
const WEATHER_MEAN: f32 = 0.556;

/// Coordinate seconds between cloud keyframes: about twenty real seconds at
/// [`crate::session::TIME_RATE`], so a server tick moves the blend a quarter of a percent.
pub const CLOUD_PERIOD_S: f64 = 2.0 * 86_400.0;

/// At the equator, meters a second; body_surface.wgsl shapes it by latitude.
const EASTERLIES_M_S: f64 = 10.0;

#[derive(Asset, TypePath, Debug, Deserialize)]
pub struct SurfaceManifest {
    /// Paths under `textures/`.
    pub classes: HashMap<Surface, String>,
    /// Baked in color, in place of the class's pattern and palette.
    #[serde(default)]
    pub bodies: HashMap<String, String>,
    #[serde(default)]
    pub clouds: HashMap<String, String>,
    pub rocky: Option<Rocky>,
    pub giants: Option<Giants>,
    pub airless: Option<Bare>,
}

#[derive(Debug, Deserialize)]
pub struct Giants {
    pub graph: String,
}

/// The graphs every rocky world with air is drawn from.
#[derive(Debug, Deserialize)]
pub struct Rocky {
    pub ground: String,
    pub clouds: String,
}

#[derive(Debug, Deserialize)]
pub struct Bare {
    pub ground: String,
}

/// What a body is painted from, before the manifest overrides it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paint {
    Climate,
    Giant,
    Airless,
    Class,
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
    /// A body named in the manifest takes its own graphs over anything its paint would give it.
    pub fn look_for(&self, name: &str, class: Surface, paint: Paint) -> Option<Look<'_>> {
        if !self.bodies.contains_key(name) {
            let alone = |graph| Some(Look { ground: Ground::Color(graph), clouds: None });
            match (paint, &self.rocky, &self.giants, &self.airless) {
                (Paint::Climate, Some(rocky), ..) => {
                    return Some(Look {
                        ground: Ground::Color(&rocky.ground),
                        clouds: Some(&rocky.clouds),
                    });
                }
                (Paint::Giant, _, Some(giants), _) => return alone(&giants.graph),
                (Paint::Airless, .., Some(bare)) => return alone(&bare.ground),
                _ => {}
            }
        }
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

/// TOML, under an extension of its own because the library's catalog already has `toml`.
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
    pub weather: [Handle<Image>; 3],
    pub climate: Handle<Image>,
    /// [`MASKS`] or [`GIANT_MASKS`], in order.
    pub masks: [Handle<Image>; 4],
    /// [`RELIEF`]; flat for anything not airless.
    pub height: Handle<Image>,
}

impl BodyImages {
    fn ids(&self) -> impl Iterator<Item = AssetId<Image>> + '_ {
        [&self.pattern, &self.color, &self.climate, &self.height]
            .into_iter()
            .chain(&self.weather)
            .chain(&self.masks)
            .map(Handle::id)
    }

    fn placeholders(images: &mut Assets<Image>) -> Self {
        let cube = Target::new(Shape::Cube(1));
        Self {
            pattern: images.add(placeholder(cube)),
            color: images.add(placeholder(cube.color())),
            weather: std::array::from_fn(|_| images.add(placeholder(WEATHER))),
            climate: images.add(placeholder(CLIMATE)),
            masks: std::array::from_fn(|_| images.add(placeholder(cube))),
            height: images.add(placeholder(Target::new(Shape::Cube(1)).format(ScalarFormat::R16Float))),
        }
    }

    pub fn material(&self, uniforms: BodySurfaceUniform) -> BodySurfaceMaterial {
        let [weather_0, weather_1, weather_2] = self.weather.clone();
        let [land, ice, growth, sand] = self.masks.clone();
        BodySurfaceMaterial {
            land,
            ice,
            growth,
            sand,
            uniforms,
            pattern: self.pattern.clone(),
            color: self.color.clone(),
            weather_0,
            weather_1,
            weather_2,
            climate: self.climate.clone(),
            height: self.height.clone(),
        }
    }
}

/// What of a body's own a material draws. All false until the manifest has said, and for a body
/// it gives nothing.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Drawn {
    pub color: bool,
    pub clouds: bool,
    /// Whether the [`MASKS`] are baked, so each band can see its own ground.
    pub grounds: bool,
    /// Whether the [`GIANT_MASKS`] are.
    pub layers: bool,
    /// Whether [`RELIEF`] is baked, so the ground is lit by its own slopes.
    pub relief: bool,
}

/// rocky.tgraph's layers that say what its ground is made of, as body_surface.wgsl's `banded`
/// mixes them: land over water, ice over everything, growth over dry land, sand over rock.
pub const MASKS: [&str; 4] = ["land", "ice", "green", "sand amount"];

/// giant.tgraph's layers, into the first three mask slots.
pub const GIANT_MASKS: [&str; 3] = ["belt", "storm", "polar"];

/// See [`BodySurfaceUniform`]'s `weather` and `drift`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Weather {
    pub weights: Vec4,
    pub drift: Vec4,
}

struct Deck {
    graph: Handle<TextureGraph>,
    seed: u32,
    /// The keyframe each weather slot was last asked to hold.
    holds: [Option<i64>; 3],
}

struct Body {
    images: BodyImages,
    class: Surface,
    climate: Option<Climate>,
    giant: Option<Giant>,
    airless: Option<Airless>,
    /// `None` until the manifest has said.
    drawn: Option<Drawn>,
    deck: Option<Deck>,
    /// What its bakes hold on the GPU once landed.
    bytes: usize,
    /// The [`Surfaces::keep`] that last found it drawn.
    wanted_at: u64,
}

/// What bodies no longer drawn may keep on the GPU, so one flown back to is not baked again. A
/// rocky world with clouds is about 48 MB. Never evicts a body being drawn.
const IDLE_BYTES: usize = if cfg!(target_arch = "wasm32") { 160 << 20 } else { 640 << 20 };

#[derive(Resource)]
pub struct Surfaces {
    manifest: Handle<SurfaceManifest>,
    /// For what is drawn with the surface material but has no surface of its own: hulls.
    pub flat: BodyImages,
    /// Kept while drawn, and afterwards within [`IDLE_BYTES`] and the same system: bodies stop
    /// and start being resolved as the ship moves, and re-baking each time would be most of the
    /// cost. Each holds its textures by strong handle, so this is what frees them.
    by_body: HashMap<String, Body>,
    star: Option<StarId>,
    keeps: u64,
}

impl FromWorld for Surfaces {
    fn from_world(world: &mut World) -> Self {
        let manifest = world.resource::<AssetServer>().load(MANIFEST);
        let flat = BodyImages::placeholders(&mut world.resource_mut::<Assets<Image>>());
        Self {
            manifest,
            flat,
            by_body: HashMap::new(),
            star: None,
            keeps: 0,
        }
    }
}

impl Surfaces {
    #[cfg(test)]
    pub(crate) fn empty(images: &mut Assets<Image>) -> Self {
        Self {
            manifest: Handle::default(),
            flat: BodyImages::placeholders(images),
            by_body: HashMap::new(),
            star: None,
            keeps: 0,
        }
    }

    /// What `name` is drawn with: flat at first, its own once baked.
    pub fn images(
        &mut self,
        name: &str,
        class: Surface,
        climate: Option<Climate>,
        giant: Option<Giant>,
        airless: Option<Airless>,
        images: &mut Assets<Image>,
    ) -> BodyImages {
        self.by_body
            .entry(name.to_owned())
            .or_insert_with(|| Body {
                images: BodyImages::placeholders(images),
                class,
                climate,
                giant,
                airless,
                drawn: None,
                deck: None,
                bytes: 0,
                wanted_at: self.keeps,
            })
            .images
            .clone()
    }

    /// Say which bodies are drawn in `star`'s system this frame, and release the rest: all of
    /// them when the system has changed, otherwise the least recently drawn past [`IDLE_BYTES`].
    pub fn keep<'a>(&mut self, star: Option<StarId>, drawn: impl IntoIterator<Item = &'a str>, bakes: &mut Bakes) {
        self.keeps += 1;
        let now = self.keeps;
        for name in drawn {
            if let Some(body) = self.by_body.get_mut(name) {
                body.wanted_at = now;
            }
        }
        let mut idle: Vec<(u64, usize, String)> = self
            .by_body
            .iter()
            .filter(|(_, b)| b.wanted_at != now)
            .map(|(name, b)| (b.wanted_at, b.bytes, name.clone()))
            .collect();
        let moved = self.star != star;
        self.star = star;
        if !moved {
            if idle.iter().map(|(_, bytes, _)| bytes).sum::<usize>() <= IDLE_BYTES {
                return;
            }
            // Most recent first: those are kept, and what is left past the budget goes.
            idle.sort_unstable_by(|a, b| b.0.cmp(&a.0));
            let mut room = IDLE_BYTES;
            let kept = idle.iter().take_while(|(_, bytes, _)| room.checked_sub(*bytes).map(|r| room = r).is_some()).count();
            idle.drain(..kept);
        }
        let mut gone = Vec::new();
        for (.., name) in idle {
            if let Some(body) = self.by_body.remove(&name) {
                gone.extend(body.images.ids());
            }
        }
        bakes.forget(&gone);
    }

    /// Whether `name` would be drawn as itself rather than flat: its graphs are chosen and their
    /// bakes have landed. True for a body nothing has asked to draw.
    pub fn ready(&self, name: &str, bakes: &Bakes) -> bool {
        let Some(body) = self.by_body.get(name) else { return true };
        let Some(drawn) = body.drawn else { return false };
        let ground = if drawn.color { &body.images.color } else { &body.images.pattern };
        bakes.settled(ground)
            && (!drawn.clouds || bakes.settled(&body.images.climate))
            && (!drawn.relief || bakes.settled(&body.images.height))
    }

    pub fn drawn(&self, name: &str) -> Drawn {
        self.by_body.get(name).and_then(|b| b.drawn).unwrap_or_default()
    }

    /// `name`'s climate, as it was when the body was first resolved.
    pub fn climate(&self, name: &str) -> Option<Climate> {
        self.by_body.get(name)?.climate
    }

    /// `name`'s cloud deck at `now_s`, coordinate time, asking for whichever keyframes it lacks.
    /// `None` until a keyframe it needs has landed, and for a body without clouds.
    pub fn weather(
        &mut self,
        name: &str,
        now_s: f64,
        radius_m: f64,
        bakes: &mut Bakes,
    ) -> Option<Weather> {
        let body = self.by_body.get_mut(name)?;
        let deck = body.deck.as_mut()?;
        let pair = blend(now_s);
        let k = pair[0].0;
        // The two being blended, and the next, baked while they are drawn.
        for j in k..k + 3 {
            let slot = slot_of(j);
            if deck.holds[slot] != Some(j) {
                deck.holds[slot] = Some(j);
                bakes.request(
                    deck.graph.clone(),
                    keyframe_seed(deck.seed, j),
                    WEATHER,
                    body.images.weather[slot].clone(),
                );
            }
        }
        let mut weights = [0.0; 3];
        let mut drift = [0.0; 3];
        for (j, weight) in pair {
            let slot = slot_of(j);
            if bakes.settled(&body.images.weather[slot]) {
                weights[slot] = weight as f32;
            }
            // Zero when drawn alone, so no keyframe shears past a period's worth.
            drift[slot] = (EASTERLIES_M_S * (now_s - j as f64 * CLOUD_PERIOD_S) / radius_m) as f32;
        }
        // A keyframe that has not landed leaves the other drawn alone, at full contrast.
        let norm = weights.iter().map(|w| w * w).sum::<f32>().sqrt();
        if norm == 0.0 {
            return None;
        }
        let [a, b, c] = weights.map(|w| w / norm);
        Some(Weather {
            weights: Vec4::new(a, b, c, WEATHER_MEAN),
            drift: Vec3::from_array(drift).extend(0.0),
        })
    }
}

/// The keyframes drawn at `now_s`, weighted so the squares sum to one: plain weights lose a third
/// of the contrast half-way.
fn blend(now_s: f64) -> [(i64, f64); 2] {
    let at = now_s / CLOUD_PERIOD_S;
    let k = at.floor();
    let angle = (at - k) * FRAC_PI_2;
    let k = k as i64;
    [(k, angle.cos()), (k + 1, angle.sin())]
}

fn slot_of(keyframe: i64) -> usize {
    keyframe.rem_euclid(3) as usize
}

fn keyframe_seed(seed: u32, keyframe: i64) -> u32 {
    seed ^ ((keyframe as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as u32
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
                body.drawn = Some(Drawn::default());
            }
        }
        return;
    };
    for (name, body) in surfaces.by_body.iter_mut().filter(|(_, b)| b.drawn.is_none()) {
        let paint = match (body.climate, body.giant, body.airless) {
            (Some(_), ..) => Paint::Climate,
            (None, Some(_), _) => Paint::Giant,
            (None, None, Some(_)) => Paint::Airless,
            (None, None, None) => Paint::Class,
        };
        let Some(look) = manifest.look_for(name, body.class, paint) else {
            warn!("no surface graph for {name} or its class, {:?}", body.class);
            body.drawn = Some(Drawn::default());
            continue;
        };
        let seed = seed_of(name);
        let graph = |path: &str| assets.load(format!("textures/{path}"));
        let drawn_as = |graph: Option<&String>| {
            graph.is_some_and(|g| matches!(look.ground, Ground::Color(p) if p == g))
        };
        let (params, masks, relief): (Params, &[&str], bool) = match (body.climate, body.giant, body.airless) {
            (Some(climate), ..) if drawn_as(manifest.rocky.as_ref().map(|r| &r.ground)) => {
                (ground_params(&climate), &MASKS, false)
            }
            (_, Some(giant), _) if drawn_as(manifest.giants.as_ref().map(|g| &g.graph)) => {
                (giant_params(&giant), &GIANT_MASKS, false)
            }
            (.., Some(airless)) if drawn_as(manifest.airless.as_ref().map(|a| &a.ground)) => {
                (airless_params(&airless), &[], true)
            }
            _ => (Params::new(), &[], false),
        };
        let mut bytes = 0;
        let mut bake = |path: &str, target: Target, image: &Handle<Image>, params: Params| {
            bytes += target.bytes();
            bakes.request_with(graph(path), seed, params, target, image.clone());
        };
        let pattern = Target::new(Shape::Cube(FACE));
        let color = Target::new(Shape::Cube(COLOR_FACE)).color();
        if let (Ground::Color(path), true) = (&look.ground, relief) {
            bake(path, RELIEF, &body.images.height, params.clone());
        }
        if let Ground::Color(path) = &look.ground {
            for (layer, image) in masks.iter().zip(&body.images.masks) {
                bake(path, pattern.layer(layer), image, params.clone());
            }
        }
        match look.ground {
            Ground::Pattern(path) => bake(path, pattern, &body.images.pattern, params),
            Ground::Color(path) => bake(path, color, &body.images.color, params),
        }
        if let Some(path) = look.clouds {
            bake(path, CLIMATE, &body.images.climate, Params::new());
            bytes += body.images.weather.len() * WEATHER.bytes();
            body.deck = Some(Deck {
                graph: graph(path),
                seed,
                holds: [None; 3],
            });
        }
        body.drawn = Some(Drawn {
            color: matches!(look.ground, Ground::Color(_)),
            clouds: look.clouds.is_some(),
            grounds: masks == MASKS,
            layers: masks == GIANT_MASKS,
            relief,
        });
        body.bytes = bytes;
    }
}

/// The share of rocky.tgraph's surface under sea across its `sea` parameter's useful range, measured over the sphere and several seeds; a seed moves it a few per cent.
/// `the_sea_covers_what_the_table_says` holds the graph to it.
const SEA_LEVELS: [(f32, f32); 11] = [
    (0.30, 0.004),
    (0.35, 0.016),
    (0.40, 0.054),
    (0.45, 0.143),
    (0.50, 0.299),
    (0.55, 0.494),
    (0.60, 0.696),
    (0.65, 0.85),
    (0.70, 0.942),
    (0.75, 0.988),
    (0.80, 0.997),
];

/// The same for its `ice`, at its default sea; `the_ice_covers_what_the_table_says`.
const ICE_LEVELS: [(f32, f32); 11] = [
    (-0.15, 0.0),
    (-0.03, 0.011),
    (0.09, 0.071),
    (0.21, 0.187),
    (0.33, 0.35),
    (0.45, 0.522),
    (0.57, 0.667),
    (0.69, 0.778),
    (0.81, 0.879),
    (0.93, 0.963),
    (1.05, 1.0),
];

/// The parameter that covers `share` of the surface, by `table`.
fn level(table: &[(f32, f32)], share: f32) -> f32 {
    let share = share.clamp(0.0, 1.0);
    let i = table
        .windows(2)
        .position(|w| share <= w[1].1)
        .unwrap_or(table.len() - 2);
    let [(a, fa), (b, fb)] = [table[i], table[i + 1]];
    if fb <= fa {
        return a;
    }
    a + (b - a) * (share - fa) / (fb - fa)
}

fn sea_level(share: f32) -> f32 {
    if share <= 0.0 { 0.0 } else { level(&SEA_LEVELS, share) }
}

/// Past the table's ends, so none is none and all is all.
fn ice_level(share: f32) -> f32 {
    match share {
        s if s <= 0.0 => -0.3,
        s if s >= 1.0 => 1.5,
        s => level(&ICE_LEVELS, s),
    }
}

/// rocky.tgraph's parameters, from what the world is.
fn ground_params(c: &Climate) -> Params {
    let scalar = ParamValue::Scalar;
    let color = |[l, c, h]: [f32; 3]| ParamValue::Color(oklcha(l, c, h, 1.0));
    vec![
        ("sea", scalar(sea_level(c.ocean))),
        ("ice", scalar(ice_level(c.ice))),
        ("life", scalar(c.life)),
        ("rust", scalar(c.rust)),
        ("sand", scalar(c.sand)),
        ("aridity", scalar(c.aridity)),
        ("dark", scalar(c.dark)),
        ("foliage", color(c.foliage.low)),
        ("foliage high", color(c.foliage.high)),
    ]
}

/// The share of airless.tgraph's surface its lava floods across its `maria` parameter,
/// measured like [`SEA_LEVELS`]; `the_maria_cover_what_the_table_says`.
const MARIA_LEVELS: [(f32, f32); 14] = [
    (0.10, 0.0),
    (0.15, 0.0),
    (0.20, 0.003),
    (0.25, 0.018),
    (0.30, 0.052),
    (0.35, 0.108),
    (0.40, 0.219),
    (0.45, 0.398),
    (0.50, 0.569),
    (0.55, 0.723),
    (0.60, 0.833),
    (0.65, 0.912),
    (0.70, 0.97),
    (0.80, 1.0),
];

fn airless_params(a: &Airless) -> Params {
    let scalar = ParamValue::Scalar;
    let color = |[l, c, h]: [f32; 3]| ParamValue::Color(oklcha(l, c, h, 1.0));
    let [ancient, later, fresh] = a.craters;
    let maria = match a.maria {
        s if s <= 0.0 => 0.0,
        s => level(&MARIA_LEVELS, s),
    };
    vec![
        ("ancient craters", scalar(ancient)),
        ("later craters", scalar(later)),
        ("fresh craters", scalar(fresh)),
        ("maria", scalar(maria)),
        ("rays", scalar(a.rays)),
        ("highland", color(a.highland)),
        ("mare", color(a.mare)),
        ("ejecta", color(a.ejecta)),
    ]
}

fn giant_params(g: &Giant) -> Params {
    let scalar = ParamValue::Scalar;
    let color = |[l, c, h]: [f32; 3]| ParamValue::Color(oklcha(l, c, h, 1.0));
    vec![
        ("bands", scalar(g.bands)),
        ("contrast", scalar(g.contrast)),
        ("turbulence", scalar(g.turbulence)),
        ("storms", scalar(g.storms)),
        ("polar", scalar(g.polar)),
        ("shift", scalar(g.shift)),
        ("spot", scalar(g.spot)),
        ("zone", color(g.colors.zone)),
        ("belt", color(g.colors.belt)),
        ("tint", color(g.colors.tint)),
        ("storm", color(g.colors.storm)),
        ("polar color", color(g.colors.polar)),
    ]
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
        for path in manifest.bodies.values() {
            baker
                .bake_color_cube(&graph(path), 8, &eval)
                .unwrap_or_else(|e| panic!("{path}: {e}"));
        }
        let rocky = manifest.rocky.as_ref().expect("rocky worlds are routed");
        let ground = graph(&rocky.ground);
        baker
            .bake_color_cube(&ground, 8, &eval)
            .unwrap_or_else(|e| panic!("{}: {e}", rocky.ground));
        for name in MASKS {
            let layer = ground.layers.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no mask {name}")).id;
            baker
                .bake_scalar_cube(&ground, layer, 8, ScalarFormat::R8Unorm, &eval)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let bare = graph(&manifest.airless.as_ref().expect("airless worlds are routed").ground);
        baker.bake_color_cube(&bare, 8, &eval).unwrap_or_else(|e| panic!("airless: {e}"));
        let height = bare.layers.iter().find(|l| Some(l.name.as_str()) == RELIEF.layer).expect("a height layer").id;
        baker
            .bake_scalar_cube(&bare, height, 8, ScalarFormat::R16Float, &eval)
            .unwrap_or_else(|e| panic!("airless height: {e}"));
        let giants = manifest.giants.as_ref().expect("giants are routed");
        let giant = graph(&giants.graph);
        baker
            .bake_color_cube(&giant, 8, &eval)
            .unwrap_or_else(|e| panic!("{}: {e}", giants.graph));
        for name in GIANT_MASKS {
            let layer = giant.layers.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no mask {name}")).id;
            baker
                .bake_scalar_cube(&giant, layer, 8, ScalarFormat::R8Unorm, &eval)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for path in manifest.clouds.values().chain([&rocky.clouds]) {
            let g = graph(path);
            for name in [WEATHER.layer, CLIMATE.layer].map(Option::unwrap) {
                let layer = g.layers.iter().find(|l| l.name == name).unwrap().id;
                baker
                    .bake_scalar_cube(&g, layer, 8, ScalarFormat::R8Unorm, &eval)
                    .unwrap_or_else(|e| panic!("{path}, {name}: {e}"));
            }
        }
    }

    /// body_surface.wgsl's `deck`, from the drive to linear gray and cover.
    fn shader_deck(drive: f32) -> (f32, f32) {
        const DENSITY_FROM: f32 = 0.58;
        const DENSITY_TO: f32 = 0.8;
        const CLOUD_KNEE: f32 = 0.4;
        const CLOUD_ALPHA: [f32; 3] = [0.0, 0.45, 0.92];
        const CLOUD_L: [f32; 3] = [0.92, 0.91, 0.94];
        let density = ((drive - DENSITY_FROM) / (DENSITY_TO - DENSITY_FROM)).clamp(0.0, 1.0);
        let (i, t) = if density < CLOUD_KNEE {
            (0, density / CLOUD_KNEE)
        } else {
            (1, (density - CLOUD_KNEE) / (1.0 - CLOUD_KNEE))
        };
        let lerp = |v: [f32; 3]| v[i] + (v[i + 1] - v[i]) * t;
        (lerp(CLOUD_L).powi(3), lerp(CLOUD_ALPHA))
    }

    /// Weather plus climate is the graph's drive, and the shader's cover and color are the
    /// graph's, so an edit to the graph the shader would not follow fails here.
    #[test]
    fn the_shaders_deck_is_the_graphs_deck() {
        let manifest = manifest();
        let g = graph(&manifest.rocky.expect("rocky worlds have clouds").clouds);
        let clouds = g.output.color.unwrap();
        let mut covered = 0;
        for seed in [1, 0xdead_beef] {
            let ctx = EvalCtx {
                seed,
                ..EvalCtx::default()
            };
            let layer = |name, s| {
                let id = g.layers.iter().find(|l| l.name == name).unwrap().id;
                eval::evaluate(&g, id, s, &ctx).l
            };
            for s in points() {
                let weather = layer(WEATHER.layer.unwrap(), s);
                let drive = weather + layer(CLIMATE.layer.unwrap(), s);
                assert!((drive - layer("drive", s)).abs() < 1e-5, "drive at {s:?}");
                let (gray, cover) = shader_deck(drive);
                let want = eval::evaluate(&g, clouds, s, &ctx);
                assert!(
                    (cover - want.alpha).abs() < 2e-3,
                    "cover at {s:?}: {cover} against {}",
                    want.alpha
                );
                assert!((gray - want.l.powi(3)).abs() < 2e-3, "gray at {s:?}");
                covered += usize::from(cover > 0.1);
            }
        }
        assert!(
            covered > 20,
            "the points hardly reached the clouds: {covered}"
        );
    }

    /// A byte clips at one, and the blend is about [`WEATHER_MEAN`].
    #[test]
    fn the_weather_has_the_mean_and_range_the_blend_assumes() {
        let g = graph(&manifest().rocky.unwrap().clouds);
        let id = g
            .layers
            .iter()
            .find(|l| Some(l.name.as_str()) == WEATHER.layer)
            .unwrap()
            .id;
        for keyframe in [0, 1, 2, -7] {
            let ctx = EvalCtx {
                seed: keyframe_seed(seed_of("Earth"), keyframe),
                ..EvalCtx::default()
            };
            let n = 32;
            let values: Vec<f32> = (0..6)
                .flat_map(|face| (0..n * n).map(move |k| (face, k)))
                .map(|(face, k)| {
                    let (u, v) = ((k % n) as f32 + 0.5, (k / n) as f32 + 0.5);
                    eval::evaluate(&g, id, cube_sample(face, u / n as f32, v / n as f32), &ctx).l
                })
                .collect();
            let mean = values.iter().sum::<f32>() / values.len() as f32;
            let max = values.iter().copied().fold(0.0, f32::max);
            assert!(
                (mean - WEATHER_MEAN).abs() < 0.015,
                "keyframe {keyframe}: mean {mean}"
            );
            assert!(max < 0.98, "keyframe {keyframe}: reaches {max}");
        }
    }

    /// No jump at a keyframe, and no loss of contrast between them.
    #[test]
    fn the_blend_is_continuous_and_keeps_its_contrast() {
        for k in [-3i64, 0, 1, 4000] {
            let at = k as f64 * CLOUD_PERIOD_S;
            let before = blend(at - 1e-3);
            let after = blend(at);
            assert_eq!(after, [(k, 1.0), (k + 1, 0.0)]);
            assert_eq!(before[1].0, k);
            assert!((before[1].1 - 1.0).abs() < 1e-6 && before[0].1.abs() < 1e-6);
            for f in [0.1, 0.5, 0.9] {
                let [(_, a), (_, b)] = blend(at + f * CLOUD_PERIOD_S);
                assert!((a * a + b * b - 1.0).abs() < 1e-12);
            }
        }
        assert_ne!(keyframe_seed(7, 0), keyframe_seed(7, 1));
        assert_eq!([0, 1, 2, -1, -3].map(slot_of), [0, 1, 2, 2, 0]);
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
            [rocky]
            ground = "worlds/rocky.tgraph"
            clouds = "worlds/clouds.tgraph"
            [giants]
            graph = "worlds/giant.tgraph"
            [airless]
            ground = "worlds/airless.tgraph"
            "#,
        )
        .unwrap();
        assert_eq!(
            manifest.look_for("Kettle e", Surface::Weathered, Paint::Climate),
            Some(Look {
                ground: Ground::Color("worlds/rocky.tgraph"),
                clouds: Some("worlds/clouds.tgraph"),
            })
        );
        assert_eq!(
            manifest.look_for("Earth", Surface::Weathered, Paint::Climate),
            Some(Look {
                ground: Ground::Color("worlds/earthlike.tgraph"),
                clouds: Some("worlds/earthlike-clouds.tgraph"),
            })
        );
        assert_eq!(
            manifest.look_for("Mercury", Surface::Weathered, Paint::Airless),
            Some(Look {
                ground: Ground::Color("worlds/airless.tgraph"),
                clouds: None,
            })
        );
        assert_eq!(
            manifest.look_for("Kettle b", Surface::GasGiant, Paint::Giant),
            Some(Look {
                ground: Ground::Color("worlds/giant.tgraph"),
                clouds: None,
            })
        );
        assert_eq!(
            manifest.look_for("Europa", Surface::Weathered, Paint::Class),
            Some(Look {
                ground: Ground::Pattern("surfaces/weathered.tgraph"),
                clouds: None,
            })
        );
        assert_eq!(manifest.look_for("Europa", Surface::Ice, Paint::Class), None);
    }

    /// giant.tgraph's masks bake as the CPU evaluates them. Outside `[0, 1]` texture-graph's GPU
    /// clamps a Map's value and its CPU caps a Multiply, so a graph that strays there passes every
    /// CPU test and draws wrong. Skipped without a GPU.
    #[test]
    fn the_giant_bakes_as_it_evaluates() {
        use lc_world::giant::{self, Inputs};
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let Ok(device) = runtime.block_on(DeviceCtx::request_headless()) else {
            eprintln!("no GPU; the giant was not baked");
            return;
        };
        let mut baker = Baker::new(device.clone());
        let g = graph(&manifest().giants.unwrap().graph);
        let paint = giant::of("Jupiter", &Inputs::from_tags(Surface::GasGiant, 1.898e27, 6.99e7, 122.0, 0.0, 5772.0, Some(35_730.0), &[]));
        let mut ctx = EvalCtx { seed: seed_of("Jupiter"), ..EvalCtx::default() };
        for (k, v) in giant_params(&paint) {
            ctx.params.insert(k.into(), v);
        }
        let resolved = g.resolve_params(&ctx);
        // Coarser, a Map's baked palette smears the narrowest ramp.
        const FACE: u32 = 256;
        const STEP: u32 = 16;
        for name in GIANT_MASKS {
            let layer = g.layers.iter().find(|l| l.name == name).unwrap().id;
            let cube = baker.bake_scalar_cube(&g, layer, FACE, ScalarFormat::R32Float, &ctx).unwrap();
            let img = texture_graph_gpu::read_scalar_volume(&device, &cube.texture, (FACE, FACE, 6), ScalarFormat::R32Float);
            for face in 0..6 {
                for y in (STEP / 2..FACE).step_by(STEP as usize) {
                    for x in (STEP / 2..FACE).step_by(STEP as usize) {
                        let (u, v) = ((x as f32 + 0.5) / FACE as f32, (y as f32 + 0.5) / FACE as f32);
                        let want = eval::evaluate(&g, layer, cube_sample(face, u, v), &resolved).l;
                        let got = img.value(x, y, face).unwrap();
                        assert!((got - want).abs() < 0.03, "{name} on face {face} at ({x}, {y}): {got} against {want}");
                    }
                }
            }
        }
    }

    /// giant.tgraph covers what `Giant::shares` says, or the survey reads a giant as another color.
    #[test]
    fn a_giant_covers_what_its_paint_says() {
        use lc_world::giant::{self, Inputs};
        let g = graph(&manifest().giants.unwrap().graph);
        let id = |name: &str| g.layers.iter().find(|l| l.name == name).unwrap().id;
        let [belt, storm, polar] = GIANT_MASKS.map(id);
        let jupiter = 1.898e27;
        let cases = [
            ("Jupiter", jupiter, 6.99e7, 122.0, 35_730.0),
            ("Saturn", 5.68e26, 5.82e7, 90.0, 38_362.0),
            ("Uranus", 8.68e25, 2.56e7, 64.0, 62_064.0),
            ("Kettle b", jupiter, 8.0e7, 1100.0, 3.0 * 86_400.0),
            ("Kettle c", 0.6 * jupiter, 6.5e7, 230.0, 14.0 * 3600.0),
            ("Kettle d", 2.0 * jupiter, 7.0e7, 140.0, 8.0 * 3600.0),
        ];
        let mut misses = Vec::new();
        for (name, mass, radius, eq, spin) in cases {
            let class = if mass >= 2.0e26 { Surface::GasGiant } else { Surface::IceGiant };
            let paint = giant::of(name, &Inputs::from_tags(class, mass, radius, eq, 0.0, 5772.0, Some(spin), &[]));
            let mut ctx = EvalCtx { seed: seed_of(name), ..EvalCtx::default() };
            for (k, v) in giant_params(&paint) {
                ctx.params.insert(k.into(), v);
            }
            let ctx = g.resolve_params(&ctx);
            let n = 24;
            let (mut sums, mut area) = ([0.0f32; 4], 0.0);
            for face in 0..6 {
                for k in 0..n * n {
                    let (u, v) = (((k % n) as f32 + 0.5) / n as f32, ((k / n) as f32 + 0.5) / n as f32);
                    let s = cube_sample(face, u, v);
                    // Solid angle: unweighted, the poles are undercounted.
                    let [a, b] = [2.0 * u - 1.0, 2.0 * v - 1.0];
                    let da = (1.0 + a * a + b * b).powf(-1.5);
                    let mask = |layer| eval::evaluate(&g, layer, s, &ctx).l.clamp(0.0, 1.0);
                    let (b, st, p) = (mask(belt), mask(storm), mask(polar));
                    let open = (1.0 - st) * (1.0 - p);
                    for (sum, w) in sums.iter_mut().zip([(1.0 - b) * open, b * open, st * (1.0 - p), p]) {
                        *sum += w * da;
                    }
                    area += da;
                }
            }
            let got = sums.map(|x| x / area);
            let want = paint.shares();
            eprintln!("{name}: {got:.3?} against {want:.3?}");
            for (k, (g, w)) in got.iter().zip(want).enumerate() {
                if (g - w).abs() > 0.03 {
                    misses.push(format!("{name} layer {k}: {g:.3} against {w:.3}"));
                }
            }
        }
        assert!(misses.is_empty(), "{misses:#?}");
    }

    /// The share of rocky.tgraph's surface where `layer` is above a half, with `name` bound to
    /// `value`, over a few seeds.
    fn share(g: &Graph, layer: &str, name: &str, value: f32) -> f32 {
        let id = g.layers.iter().find(|l| l.name == layer).unwrap().id;
        let n = 24;
        let mut over = 0;
        let mut all = 0;
        for seed in [1, 7, 0xdead_beef] {
            let mut ctx = EvalCtx { seed, ..EvalCtx::default() };
            ctx.params.insert(name.into(), texture_graph_core::ParamValue::Scalar(value));
            let ctx = g.resolve_params(&ctx);
            for face in 0..6 {
                for k in 0..n * n {
                    let (u, v) = ((k % n) as f32 + 0.5, (k / n) as f32 + 0.5);
                    let s = cube_sample(face, u / n as f32, v / n as f32);
                    over += usize::from(eval::evaluate(g, id, s, &ctx).l > 0.5);
                    all += 1;
                }
            }
        }
        over as f32 / all as f32
    }

    /// A table's shares against the graph's, and its inverse against both.
    fn holds(table: &[(f32, f32)], measure: impl Fn(f32) -> f32, invert: impl Fn(f32) -> f32) {
        let measured: Vec<(f32, f32)> = table.iter().map(|&(p, _)| (p, measure(p))).collect();
        eprintln!("{measured:?}");
        for (&(p, want), (_, got)) in table.iter().zip(&measured) {
            assert!((want - got).abs() < 0.02, "at {p}: {got} against {want}");
        }
        assert!(table.windows(2).all(|w| w[1].1 >= w[0].1));
        for share in [0.1, 0.5, 0.71, 0.95] {
            let got = measure(invert(share));
            assert!((got - share).abs() < 0.04, "{share} of the surface: {got}");
        }
    }

    #[test]
    fn the_sea_covers_what_the_table_says() {
        let g = graph(&manifest().rocky.unwrap().ground);
        holds(&SEA_LEVELS, |sea| 1.0 - share(&g, "land", "sea", sea), sea_level);
    }

    #[test]
    fn the_ice_covers_what_the_table_says() {
        let g = graph(&manifest().rocky.unwrap().ground);
        holds(&ICE_LEVELS, |ice| share(&g, "ice", "ice", ice), ice_level);
        assert_eq!(share(&g, "ice", "ice", ice_level(0.0)), 0.0, "no ice is none");
        assert_eq!(share(&g, "ice", "ice", ice_level(1.0)), 1.0, "all ice is all");
    }

    #[test]
    fn the_maria_cover_what_the_table_says() {
        let g = graph(&manifest().airless.unwrap().ground);
        holds(&MARIA_LEVELS, |maria| share(&g, "mare", "maria", maria), |share| level(&MARIA_LEVELS, share));
    }

    /// The shader assumes [`RELIEF_SCALE`] for every series.
    #[test]
    fn the_relief_is_drawn_at_the_scale_the_shader_assumes() {
        let g = graph(&manifest().airless.unwrap().ground);
        let mut series = 0;
        for layer in &g.layers {
            if let texture_graph_core::LayerKind::Craters(c) = &layer.kind {
                if c.output == texture_graph_core::CraterOutput::Height {
                    assert_eq!(c.relief, RELIEF_SCALE, "{}", layer.name);
                    series += 1;
                }
            }
        }
        assert_eq!(series, 3);
    }

    fn held(names: &[&str], images: &mut Assets<Image>) -> Surfaces {
        let mut surfaces = Surfaces::empty(images);
        for name in names {
            surfaces.images(name, Surface::Rock, None, None, None, images);
            surfaces.by_body.get_mut(*name).unwrap().bytes = IDLE_BYTES / 2;
        }
        surfaces
    }

    fn kept(surfaces: &Surfaces) -> Vec<&str> {
        let mut names: Vec<&str> = surfaces.by_body.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn a_new_system_releases_what_the_last_one_drew() {
        let mut images = Assets::<Image>::default();
        let mut bakes = Bakes::default();
        let mut surfaces = held(&["Earth", "Mars"], &mut images);
        let (sol, other) = (StarId::synthesize("test", 1), StarId::synthesize("test", 2));
        surfaces.keep(Some(sol), ["Earth", "Mars"], &mut bakes);
        surfaces.keep(Some(sol), [], &mut bakes);
        assert_eq!(kept(&surfaces), ["Earth", "Mars"], "within the budget and the same system");

        surfaces.images("Proxima b", Surface::Rock, None, None, None, &mut images);
        surfaces.keep(Some(other), ["Proxima b"], &mut bakes);
        assert_eq!(kept(&surfaces), ["Proxima b"]);

        surfaces.keep(None, [], &mut bakes);
        assert!(kept(&surfaces).is_empty(), "between stars nothing is drawn");
    }

    #[test]
    fn idle_bodies_past_the_budget_go_least_recently_drawn_first() {
        let mut images = Assets::<Image>::default();
        let mut bakes = Bakes::default();
        let mut surfaces = held(&["Io", "Europa", "Ganymede", "Callisto"], &mut images);
        let jupiter = Some(StarId::synthesize("test", 1));
        let moons = ["Io", "Europa", "Ganymede", "Callisto"];
        for first in 0..moons.len() {
            surfaces.keep(jupiter, moons[first..].iter().copied(), &mut bakes);
        }
        // Each is half the budget. Callisto is drawn; Ganymede and Europa, drawn last, fill it.
        assert_eq!(kept(&surfaces), ["Callisto", "Europa", "Ganymede"]);

        surfaces.keep(jupiter, [], &mut bakes);
        assert_eq!(kept(&surfaces), ["Callisto", "Ganymede"]);
    }

    #[test]
    fn a_drawn_body_is_never_released() {
        let mut images = Assets::<Image>::default();
        let mut bakes = Bakes::default();
        let names = ["Io", "Europa", "Ganymede", "Callisto"];
        let mut surfaces = held(&names, &mut images);
        let other = Some(StarId::synthesize("test", 2));
        surfaces.keep(other, names, &mut bakes);
        assert_eq!(kept(&surfaces).len(), 4, "twice the budget, all of it drawn");
    }
}
