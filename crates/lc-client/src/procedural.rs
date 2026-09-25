//! Procedural textures: graphs authored in texture-graph, baked on the GPU at load.
//!
//! A graph is an asset like any other, so in a browser it arrives from the CDN beside the
//! shaders. The bake runs on Bevy's own device, then reads back into an ordinary [`Image`] —
//! Bevy 0.19 has no way to adopt a foreign `wgpu::Texture` as an image, and a single-channel
//! field is small enough that the round trip costs nothing worth avoiding.
//!
//! Whoever wants a bake adds a flat [`placeholder`] image, binds it, and hands its handle to
//! [`Bakes::request`]; the bake replaces the image behind that handle when it lands. So a
//! material drawn before then draws unchanged but for the texture, and a bake that fails leaves
//! a texture missing rather than a game that cannot start. See `lightcone/docs/07-rendering.md`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, LoadState, RenderAssetUsages};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, PollType, TextureDimension, TextureFormat, TextureViewDescriptor,
    TextureViewDimension,
};
use bevy::render::renderer::{RenderAdapter, RenderDevice, RenderQueue};
use em_render::body_surface_material::BodySurfaceMaterial;
use em_render::plume_material::PlumeMaterial;
use em_render::population_material::PopulationMaterial;
use em_render::relativistic_starfield_material::RelativisticStarfieldMaterial;
use texture_graph_core::{CUBE_FACES, EvalCtx, Graph, LoadError, ParamValue, load_from_str};
use texture_graph_gpu::{
    Baker, DeviceCtx, ScalarFormat, read_rgba8_async, read_rgba8_layers_async,
    read_scalar_volume_async,
};

/// Where the population grain graph lives under the asset root.
const POPULATION_GRAIN: &str = "textures/population_grain.tgraph";

const CORONA: &str = "textures/corona.tgraph";

const PLUME: &str = "textures/plume.tgraph";

/// Texels along each edge of the plume's churn: six per cell across
/// [`em_render::plume_material::CHURN_PERIOD`] cells. Measured against the graph evaluated
/// exactly, six keeps the lanes within about one per cent, for seven megabytes; eight halves that
/// for seventeen.
const CHURN_TEXELS: u32 = 192;

/// Texels along a corona cubemap face's edge. A corona samples a band about the great circle
/// facing the viewer, about two thousand texels round, with its finest octave some five texels
/// a cycle.
const CORONA_FACE: u32 = 512;

/// Texels along each edge of the grain volume: eight per grain across
/// [`em_render::population_material::GRAIN_TILE`] grains. Two megabytes at one byte a texel.
const GRAIN_TEXELS: u32 = 128;

/// A texture graph, as the asset server delivers it.
#[derive(Asset, TypePath, Debug)]
pub struct TextureGraph(pub Graph);

#[derive(Debug)]
pub enum TextureGraphLoadError {
    Io(std::io::Error),
    Utf8(std::string::FromUtf8Error),
    Graph(LoadError),
}

impl std::fmt::Display for TextureGraphLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Utf8(e) => write!(f, "{e}"),
            Self::Graph(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for TextureGraphLoadError {}

#[derive(Default, TypePath)]
pub struct TextureGraphLoader;

impl AssetLoader for TextureGraphLoader {
    type Asset = TextureGraph;
    type Settings = ();
    type Error = TextureGraphLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load: &mut LoadContext<'_>,
    ) -> Result<TextureGraph, Self::Error> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(TextureGraphLoadError::Io)?;
        let text = String::from_utf8(bytes).map_err(TextureGraphLoadError::Utf8)?;
        let file = load_from_str(&text).map_err(TextureGraphLoadError::Graph)?;
        Ok(TextureGraph(file.graph))
    }

    fn extensions(&self) -> &[&str] {
        &["tgraph"]
    }
}

/// What a bake fills.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A repeating volume, `n` texels a side, sampled by position.
    Volume(u32),
    /// A cubemap, `n` texels a face edge, sampled by direction: the graph on a sphere.
    Cube(u32),
    /// A repeating square, `n` texels a side: the graph's unit square as one tile.
    Plane(u32),
}

/// What each texel holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Format {
    /// One byte a texel unless a field leaves `[0, 1]`: the population grain was compared at
    /// sixteen bits and came out identical.
    Scalar(ScalarFormat),
    /// The graph's color output, sRGB with straight alpha. On a sphere or a plane.
    Color,
}

/// What a bake fills, and from which layer of the graph.
#[derive(Copy, Clone, Debug)]
pub struct Target {
    pub shape: Shape,
    pub format: Format,
    /// `None` is the graph's output. A graph can carry several fields side by side, and each is
    /// baked on its own.
    pub layer: Option<&'static str>,
}

impl Target {
    pub const fn new(shape: Shape) -> Self {
        Self {
            shape,
            format: Format::Scalar(ScalarFormat::R8Unorm),
            layer: None,
        }
    }

    pub const fn layer(self, name: &'static str) -> Self {
        Self {
            layer: Some(name),
            ..self
        }
    }

    pub const fn format(self, format: ScalarFormat) -> Self {
        Self {
            format: Format::Scalar(format),
            ..self
        }
    }

    pub const fn color(self) -> Self {
        Self {
            format: Format::Color,
            ..self
        }
    }

    /// What the baked texture holds on the GPU.
    pub const fn bytes(self) -> usize {
        let texels = match self.shape {
            Shape::Volume(n) => n as usize * n as usize * n as usize,
            Shape::Cube(n) => n as usize * n as usize * CUBE_FACES as usize,
            Shape::Plane(n) => n as usize * n as usize,
        };
        let per_texel = match self.format {
            Format::Scalar(ScalarFormat::R8Unorm) => 1,
            Format::Scalar(ScalarFormat::R16Float) => 2,
            Format::Scalar(ScalarFormat::R32Float) | Format::Color => 4,
        };
        texels * per_texel
    }
}

/// A flat image of `target`'s kind, half-way everywhere, which every pattern here reads as no
/// pattern at all; a color is a transparent gray. What a material binds until its bake lands.
pub fn placeholder(target: Target) -> Image {
    let shape = match target.shape {
        Shape::Volume(_) => Shape::Volume(1),
        Shape::Cube(_) => Shape::Cube(1),
        Shape::Plane(_) => Shape::Plane(1),
    };
    let texels = match shape {
        Shape::Volume(_) | Shape::Plane(_) => 1,
        Shape::Cube(_) => CUBE_FACES as usize,
    };
    let half: &[u8] = match target.format {
        Format::Scalar(ScalarFormat::R8Unorm) => &[128],
        Format::Scalar(ScalarFormat::R16Float) => &0x3800u16.to_le_bytes(),
        Format::Scalar(ScalarFormat::R32Float) => &0.5f32.to_le_bytes(),
        Format::Color => &[128, 128, 128, 0],
    };
    image_of(Target { shape, ..target }, half.repeat(texels))
}

fn image_of(target: Target, bytes: Vec<u8>) -> Image {
    let shape = target.shape;
    let (size, dimension) = match shape {
        Shape::Volume(n) => (
            Extent3d {
                width: n,
                height: n,
                depth_or_array_layers: n,
            },
            TextureDimension::D3,
        ),
        Shape::Cube(n) => (
            Extent3d {
                width: n,
                height: n,
                depth_or_array_layers: CUBE_FACES,
            },
            TextureDimension::D2,
        ),
        Shape::Plane(n) => (
            Extent3d {
                width: n,
                height: n,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
        ),
    };
    let mut image = Image::new(
        size,
        dimension,
        bytes,
        match target.format {
            Format::Scalar(ScalarFormat::R8Unorm) => TextureFormat::R8Unorm,
            Format::Scalar(ScalarFormat::R16Float) => TextureFormat::R16Float,
            Format::Scalar(ScalarFormat::R32Float) => TextureFormat::R32Float,
            // The bake encodes sRGB itself, so the sampler decodes to linear.
            Format::Color => TextureFormat::Rgba8UnormSrgb,
        },
        RenderAssetUsages::RENDER_WORLD,
    );
    // A volume repeats, which the population grain depends on. A cube has no edge to repeat.
    let wrap = match shape {
        Shape::Volume(_) | Shape::Plane(_) => ImageAddressMode::Repeat,
        Shape::Cube(_) => ImageAddressMode::ClampToEdge,
    };
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: wrap,
        address_mode_v: wrap,
        address_mode_w: wrap,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    if let Shape::Cube(_) = shape {
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        });
    }
    image
}

/// Bindings for a graph's parameters. One left out takes its declared default.
pub type Params = Vec<(&'static str, ParamValue)>;

/// The texels, in the target's format.
type Readback = Pin<Box<dyn Future<Output = Vec<u8>> + Send>>;

struct Request {
    graph: Handle<TextureGraph>,
    seed: u32,
    params: Params,
    target: Target,
    image: Handle<Image>,
}

struct Reading {
    /// Behind a mutex only because a resource must be `Sync` and a boxed future is not.
    readback: Mutex<Readback>,
    target: Target,
    image: Handle<Image>,
}

/// Bakes waiting on their graph or the device, and bakes waiting on the GPU.
#[derive(Resource, Default)]
pub struct Bakes {
    waiting: Vec<Request>,
    reading: Vec<Reading>,
    /// Bakes asked of each image and not yet settled, landed or failed.
    unsettled: HashMap<AssetId<Image>, u32>,
    /// Built once: its pipelines are most of what a first bake costs.
    baker: Option<(Baker, RenderDevice)>,
}

impl Bakes {
    /// Bake `graph` into the image behind `image`, with `seed` as the graph's global seed.
    pub fn request(
        &mut self,
        graph: Handle<TextureGraph>,
        seed: u32,
        target: Target,
        image: Handle<Image>,
    ) {
        self.request_with(graph, seed, Params::new(), target, image);
    }

    /// The same, with the graph's parameters bound.
    pub fn request_with(
        &mut self,
        graph: Handle<TextureGraph>,
        seed: u32,
        params: Params,
        target: Target,
        image: Handle<Image>,
    ) {
        *self.unsettled.entry(image.id()).or_default() += 1;
        self.waiting.push(Request {
            graph,
            seed,
            params,
            target,
            image,
        });
    }

    /// Drop what is still waiting to bake into `images`, whose owner no longer wants them. A bake
    /// already on the GPU lands anyway, into an image nothing else holds.
    pub fn forget(&mut self, images: &[AssetId<Image>]) {
        self.waiting.retain(|request| !images.contains(&request.image.id()));
        for image in images {
            self.unsettled.remove(image);
        }
    }

    /// Whether every bake asked of `image` has landed or failed. Bakes of one image land in the
    /// order they were asked for, so the image then holds the last.
    pub fn settled(&self, image: &Handle<Image>) -> bool {
        !self.unsettled.contains_key(&image.id())
    }
}

/// A free function so it can run while the baker is borrowed out of [`Bakes`].
fn settle(unsettled: &mut HashMap<AssetId<Image>, u32>, image: &Handle<Image>) {
    if let Some(n) = unsettled.get_mut(&image.id()) {
        *n -= 1;
        if *n == 0 {
            unsettled.remove(&image.id());
        }
    }
}

/// Start what can start, and finish what the GPU has finished.
///
/// Neither the graphs nor the device exist at startup: a graph is a fetch in a browser, and
/// Bevy may still be bringing its renderer up.
fn run_bakes(
    mut bakes: ResMut<Bakes>,
    graphs: Res<Assets<TextureGraph>>,
    assets: Res<AssetServer>,
    device: Option<Res<RenderDevice>>,
    queue: Option<Res<RenderQueue>>,
    adapter: Option<Res<RenderAdapter>>,
    mut images: ResMut<Assets<Image>>,
    mut populations: ResMut<Assets<PopulationMaterial>>,
    mut surfaces: ResMut<Assets<BodySurfaceMaterial>>,
    mut skies: ResMut<Assets<RelativisticStarfieldMaterial>>,
    mut plumes: ResMut<Assets<PlumeMaterial>>,
) {
    let bakes = &mut *bakes;
    if bakes.baker.is_none() {
        let (Some(device), Some(queue), Some(adapter)) = (device, queue, adapter) else {
            return;
        };
        let ctx = DeviceCtx::from_shared(
            Arc::new((**adapter.0).clone()),
            Arc::new(device.wgpu_device().clone()),
            Arc::new((**queue.0).clone()),
        );
        bakes.baker = Some((Baker::new(ctx), device.clone()));
    }
    let (baker, device) = bakes.baker.as_mut().expect("built above");

    let waiting = std::mem::take(&mut bakes.waiting);
    for request in waiting {
        if let LoadState::Failed(e) = assets.load_state(&request.graph) {
            warn!("a procedural texture's graph did not load, drawing without it: {e}");
            settle(&mut bakes.unsettled, &request.image);
            continue;
        }
        let Some(TextureGraph(graph)) = graphs.get(&request.graph) else {
            bakes.waiting.push(request);
            continue;
        };
        match start(baker, graph, request.seed, &request.params, request.target) {
            Ok(readback) => bakes.reading.push(Reading {
                readback: Mutex::new(readback),
                target: request.target,
                image: request.image,
            }),
            Err(e) => {
                warn!("a procedural texture did not bake, drawing without it: {e}");
                settle(&mut bakes.unsettled, &request.image);
            }
        }
    }

    if bakes.reading.is_empty() {
        return;
    }
    // Fires the map callbacks on native. A browser fires them from its own event loop and this
    // is a no-op there.
    let _ = device.poll(PollType::Poll);
    let mut landed = Vec::new();
    bakes.reading.retain(|reading| {
        let polled = reading
            .readback
            .lock()
            .unwrap()
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()));
        let Poll::Ready(bytes) = polled else {
            return true;
        };
        debug!("baked a procedural texture: {:?}", reading.target);
        let _ = images.insert(&reading.image, image_of(reading.target, bytes));
        landed.push(reading.image.clone());
        false
    });
    for image in &landed {
        settle(&mut bakes.unsettled, image);
    }
    if !landed.is_empty() {
        // Visiting every material marks it changed, which is what makes Bevy rebuild the bind
        // groups that still hold a placeholder's view.
        for _ in populations.iter_mut() {}
        for _ in surfaces.iter_mut() {}
        for _ in skies.iter_mut() {}
        for _ in plumes.iter_mut() {}
    }
}

fn start(
    baker: &mut Baker,
    graph: &Graph,
    seed: u32,
    params: &Params,
    target: Target,
) -> Result<Readback, String> {
    let eval = EvalCtx {
        seed,
        params: params
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect(),
        ..EvalCtx::default()
    };
    let format = match target.format {
        Format::Scalar(format) => format,
        Format::Color => {
            if target.layer.is_some() {
                return Err("a color bake is of a graph's output".into());
            }
            let n = match target.shape {
                Shape::Cube(n) => n,
                Shape::Plane(n) => {
                    // Straight alpha, not composited over the preview's checker.
                    let out = baker
                        .bake_output(graph, (n, n), &eval, true)
                        .map_err(|e| e.to_string())?;
                    let ctx = baker.ctx().clone();
                    return Ok(Box::pin(async move {
                        read_rgba8_async(&ctx, &out.color, (n, n)).await.pixels
                    }));
                }
                Shape::Volume(_) => return Err("a color bake is on a sphere or a plane".into()),
            };
            let cube = baker
                .bake_color_cube(graph, n, &eval)
                .map_err(|e| e.to_string())?;
            let ctx = baker.ctx().clone();
            return Ok(Box::pin(async move {
                read_rgba8_layers_async(&ctx, &cube.texture, (n, n, CUBE_FACES))
                    .await
                    .pixels
            }));
        }
    };
    let layer = match target.layer {
        Some(name) => graph
            .layers
            .iter()
            .find(|l| l.name == name)
            .map(|l| l.id)
            .ok_or(format!("the graph has no layer named {name:?}"))?,
        None => graph
            .output
            .color
            .ok_or("the graph's output has no color layer")?,
    };
    let ctx = baker.ctx().clone();
    let (texture, size) = match target.shape {
        Shape::Volume(n) => {
            let volume = baker
                .bake_scalar_volume(graph, layer, n, n, format, &eval)
                .map_err(|e| e.to_string())?;
            (volume.texture, volume.size)
        }
        Shape::Cube(n) => {
            let cube = baker
                .bake_scalar_cube(graph, layer, n, format, &eval)
                .map_err(|e| e.to_string())?;
            (cube.texture, (n, n, CUBE_FACES))
        }
        Shape::Plane(n) => {
            let texture = baker
                .bake_scalar(graph, layer, (n, n), format, &eval)
                .map_err(|e| e.to_string())?;
            (texture, (n, n, 1))
        }
    };
    Ok(Box::pin(async move {
        read_scalar_volume_async(&ctx, &texture, size, format)
            .await
            .bytes
    }))
}

/// The grain every population and ring samples.
#[derive(Resource)]
pub struct PopulationGrain {
    pub image: Handle<Image>,
}

impl FromWorld for PopulationGrain {
    fn from_world(world: &mut World) -> Self {
        let graph = world.resource::<AssetServer>().load(POPULATION_GRAIN);
        let target = Target::new(Shape::Volume(GRAIN_TEXELS));
        let image = world
            .resource_mut::<Assets<Image>>()
            .add(placeholder(target));
        world.resource_mut::<Bakes>().request(
            graph,
            EvalCtx::default().seed,
            target,
            image.clone(),
        );
        Self { image }
    }
}

/// The churn every drive's exhaust streaks with. One for every plume: each craft's own streaks
/// come from its phase, not from its texture.
#[derive(Resource)]
pub struct PlumeChurn {
    pub image: Handle<Image>,
}

impl FromWorld for PlumeChurn {
    fn from_world(world: &mut World) -> Self {
        let graph = world.resource::<AssetServer>().load(PLUME);
        let target = Target::new(Shape::Volume(CHURN_TEXELS));
        let image = world
            .resource_mut::<Assets<Image>>()
            .add(placeholder(target));
        world.resource_mut::<Bakes>().request(
            graph,
            EvalCtx::default().seed,
            target,
            image.clone(),
        );
        Self { image }
    }
}

/// The corona's two fields, baked once and shared by every star.
#[derive(Resource)]
pub struct Corona {
    pub filaments: Handle<Image>,
    pub reach: Handle<Image>,
}

impl FromWorld for Corona {
    fn from_world(world: &mut World) -> Self {
        let graph: Handle<TextureGraph> = world.resource::<AssetServer>().load(CORONA);
        let cube = Target::new(Shape::Cube(CORONA_FACE));
        // Sixteen bits for the threads: their sum peaks past one, where a byte would clip.
        let filaments = cube.layer("filaments").format(ScalarFormat::R16Float);
        let reach = cube.layer("reach");
        let mut images = world.resource_mut::<Assets<Image>>();
        let corona = Self {
            filaments: images.add(placeholder(filaments)),
            reach: images.add(placeholder(reach)),
        };
        // One seed for the bake; each star turns the result by its own. See `spin_of` in
        // starfield.wgsl.
        let seed = EvalCtx::default().seed;
        let mut bakes = world.resource_mut::<Bakes>();
        bakes.request(graph.clone(), seed, filaments, corona.filaments.clone());
        bakes.request(graph, seed, reach, corona.reach.clone());
        corona
    }
}

pub struct ProceduralTexturesPlugin;

impl Plugin for ProceduralTexturesPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<TextureGraph>()
            .init_asset_loader::<TextureGraphLoader>()
            .init_resource::<Bakes>()
            .init_resource::<PopulationGrain>()
            .init_resource::<Corona>()
            .init_resource::<PlumeChurn>()
            .add_plugins(crate::surfaces::SurfacesPlugin)
            .add_systems(Update, run_bakes);
    }
}

#[cfg(test)]
mod tests {
    use em_render::population_material::GRAIN_TILE;
    use texture_graph_core::{
        BlendMode, FractalMode, LayerKind, Noise, NoiseDims, NoiseKernel, NoiseRange, Sample,
        cube_sample, eval,
    };

    use super::*;

    #[test]
    fn a_forgotten_image_is_not_baked_and_counts_as_settled() {
        let mut images = Assets::<Image>::default();
        let mut bakes = Bakes::default();
        let target = Target::new(Shape::Cube(4));
        let (kept, forgotten) = (images.add(placeholder(target)), images.add(placeholder(target)));
        bakes.request(Handle::default(), 1, target, kept.clone());
        bakes.request(Handle::default(), 1, target, forgotten.clone());
        bakes.forget(&[forgotten.id()]);
        assert_eq!(bakes.waiting.len(), 1);
        assert_eq!(bakes.waiting[0].image, kept);
        assert!(bakes.settled(&forgotten) && !bakes.settled(&kept));
    }

    #[test]
    fn a_target_knows_its_size() {
        assert_eq!(Target::new(Shape::Cube(1024)).color().bytes(), 6 * 1024 * 1024 * 4);
        assert_eq!(Target::new(Shape::Cube(512)).bytes(), 6 * 512 * 512);
        let relief = Target::new(Shape::Cube(1024)).format(ScalarFormat::R16Float);
        assert_eq!(relief.bytes(), 6 * 1024 * 1024 * 2);
    }

    fn shipped(path: &str) -> Graph {
        let full = format!("{}/assets/{path}", env!("CARGO_MANIFEST_DIR"));
        load_from_str(&std::fs::read_to_string(&full).unwrap())
            .unwrap()
            .graph
    }

    fn layer<'g>(g: &'g Graph, name: &str) -> &'g LayerKind {
        &g.layers
            .iter()
            .find(|l| l.name == name)
            .unwrap_or_else(|| panic!("no {name}"))
            .kind
    }

    fn noise<'g>(g: &'g Graph, name: &str) -> &'g Noise {
        let LayerKind::Noise(n) = layer(g, name) else {
            panic!("{name} is not noise")
        };
        n
    }

    /// starfield.wgsl's corona as it stood, which `corona.tgraph` has to stay: three octaves of
    /// squared ridges at lacunarity 2.4 and gain 0.42, from amplitude 0.66, and a reach field on
    /// the same scale with its own seed. Frequency 24 in graph space is the shader's 12 per
    /// radian, because the graph samples at `dir / 2 + 1/2`.
    #[test]
    fn the_shipped_corona_is_the_shaders_corona() {
        let g = shipped(CORONA);
        let ridges = noise(&g, "ridges");
        assert_eq!(
            (ridges.dims, ridges.kernel, ridges.range),
            (NoiseDims::D3, NoiseKernel::Value, NoiseRange::Unsigned)
        );
        assert_eq!(ridges.frequency, 24.0);
        let f = ridges.fractal;
        assert_eq!(
            (f.octaves, f.lacunarity, f.gain, f.mode, f.normalize),
            (3, 2.4, 0.42, FractalMode::Ridged, false)
        );

        let reach = noise(&g, "reach");
        assert_eq!(
            (
                reach.kernel,
                reach.range,
                reach.frequency,
                reach.fractal.octaves
            ),
            (NoiseKernel::Value, NoiseRange::Unsigned, 24.0, 1)
        );
        assert_ne!(
            reach.seed_offset, ridges.seed_offset,
            "length and brightness must not be one field"
        );

        let LayerKind::Mix(m) = layer(&g, "filaments") else {
            panic!("filaments is not a mix")
        };
        assert_eq!(m.mode, BlendMode::Blend);
        let ridges_id = g.layers.iter().find(|l| l.name == "ridges").unwrap().id;
        let filaments_id = g.layers.iter().find(|l| l.name == "filaments").unwrap().id;
        let ctx = EvalCtx::default();
        let mut peak = 0.0f32;
        for face in 0..CUBE_FACES {
            for k in 0..64 {
                let s = cube_sample(face, (k % 8) as f32 / 7.0, (k / 8) as f32 / 7.0);
                let r = eval::evaluate(&g, ridges_id, s, &ctx).l;
                let got = eval::evaluate(&g, filaments_id, s, &ctx).l;
                assert!(
                    (got - 0.66 * r).abs() < 1e-4,
                    "filaments at {s:?}: {got}, against 0.66 × {r}"
                );
                peak = peak.max(got);
            }
        }
        // Why the threads bake at sixteen bits.
        assert!(
            peak <= 0.66 * (1.0 + 0.42 + 0.42 * 0.42) + 1e-4,
            "filaments peak at {peak}"
        );
    }

    /// plume.wgsl's churn as it stood — two octaves at lacunarity 2, gain 0.5, normalized — and
    /// what the shader and the host assume of it: that it repeats on every axis at
    /// [`CHURN_PERIOD`], which is where the host wraps the phase. A period that disagreed would
    /// make the streaks jump once every wrap, which no test of the shader could see.
    #[test]
    fn the_shipped_churn_repeats_where_the_host_wraps() {
        use em_render::plume_material::CHURN_PERIOD;
        let g = shipped(PLUME);
        let n = noise(&g, "churn");
        assert_eq!(
            g.output.color,
            g.layers.iter().find(|l| l.name == "churn").map(|l| l.id)
        );
        assert_eq!(
            (n.dims, n.kernel, n.range),
            (NoiseDims::D3, NoiseKernel::Value, NoiseRange::Unsigned)
        );
        assert_eq!(n.frequency, CHURN_PERIOD);
        assert_eq!(n.period, [CHURN_PERIOD as u32; 3]);
        let f = n.fractal;
        assert_eq!(
            (f.octaves, f.lacunarity, f.gain, f.mode, f.normalize),
            (2, 2.0, 0.5, FractalMode::Standard, true)
        );
        // And the bake spends a whole number of texels on each cell, so a texel boundary falls
        // on every lattice line.
        assert_eq!(CHURN_TEXELS % CHURN_PERIOD as u32, 0);
    }

    /// What the shader assumes of the shipped grain, which an edit in the editor could break
    /// without anything else noticing: the volume must tile on the unit cube, with as many
    /// grains across it as the shader divides by, centered on half.
    #[test]
    fn the_shipped_grain_tiles_as_the_shader_assumes() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/textures/population_grain.tgraph"
        );
        let graph = load_from_str(&std::fs::read_to_string(path).unwrap())
            .unwrap()
            .graph;
        let layer = graph
            .get(graph.output.color.expect("an output layer"))
            .unwrap();
        let LayerKind::Noise(noise) = &layer.kind else {
            panic!(
                "the grain's output is a {}, not noise",
                layer.kind.category_label()
            );
        };
        assert_eq!(
            noise.kernel,
            NoiseKernel::Value,
            "only the value kernel tiles"
        );
        assert_eq!(
            noise.range,
            NoiseRange::Unsigned,
            "the shader recenters on half itself"
        );
        assert_eq!(noise.frequency, GRAIN_TILE);
        assert_eq!(
            noise.period, [GRAIN_TILE as u32; 3],
            "seamless on the cube is period == frequency"
        );
    }

    /// What hull.wgsl assumes of every hull graph: albedo on the output, a `lights` layer beside
    /// it, and both repeating on the unit square, or every tile's edge is a seam across the hull.
    #[test]
    fn every_hull_graph_tiles_with_albedo_and_lights() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/textures/hull");
        let mut seen = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let g = load_from_str(&std::fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                .graph;
            let albedo = g.output.color.expect("albedo on the output");
            let lights = g.layers.iter().find(|l| l.name == "lights").expect("a lights layer").id;
            let ctx = EvalCtx::default();
            for layer in [albedo, lights] {
                // Low-discrepancy, not a grid: a grid shares the tile's own pitches and can land
                // every point between the windows.
                for k in 0..256 {
                    let u = (k as f32 * 0.618_034).fract();
                    let v = (k as f32 * 0.754_878).fract();
                    let at = |u, v| eval::evaluate(&g, layer, Sample::new(u, v, 0.5), &ctx);
                    let here = at(u, v);
                    for there in [at(u + 1.0, v), at(u, v + 1.0)] {
                        assert!(
                            (here.l - there.l).abs() < 1e-4,
                            "{} does not repeat at ({u}, {v}): {} against {}",
                            path.display(),
                            here.l,
                            there.l
                        );
                    }
                }
            }
            seen += 1;
        }
        assert_eq!(seen, 8, "one graph a kind");
    }
}
