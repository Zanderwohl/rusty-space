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
use em_render::population_material::PopulationMaterial;
use texture_graph_core::{CUBE_FACES, EvalCtx, Graph, LoadError, load_from_str};
use texture_graph_gpu::{Baker, DeviceCtx, ScalarFormat, ScalarImage, read_scalar_volume_async};

/// Where the population grain graph lives under the asset root.
const POPULATION_GRAIN: &str = "textures/population_grain.tgraph";

/// Texels along each edge of the grain volume: eight per grain across
/// [`em_render::population_material::GRAIN_TILE`] grains. Two megabytes at one byte a texel.
const GRAIN_TEXELS: u32 = 128;

/// One byte a texel for every bake. Every field here is a pattern in `[0, 1]`; the population
/// grain was compared at sixteen bits and came out identical.
const FORMAT: ScalarFormat = ScalarFormat::R8Unorm;

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
}

/// A flat image of `shape`'s kind, half-way everywhere, which every pattern here reads as no
/// pattern at all. What a material binds until its bake lands.
pub fn placeholder(shape: Shape) -> Image {
    let shape = match shape {
        Shape::Volume(_) => Shape::Volume(1),
        Shape::Cube(_) => Shape::Cube(1),
    };
    let texels = match shape {
        Shape::Volume(_) => 1,
        Shape::Cube(_) => CUBE_FACES as usize,
    };
    image_of(shape, vec![128; texels])
}

fn image_of(shape: Shape, bytes: Vec<u8>) -> Image {
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
    };
    let mut image = Image::new(
        size,
        dimension,
        bytes,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    // A volume repeats, which the population grain depends on. A cube has no edge to repeat.
    let wrap = match shape {
        Shape::Volume(_) => ImageAddressMode::Repeat,
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

type Readback = Pin<Box<dyn Future<Output = ScalarImage> + Send>>;

struct Request {
    graph: Handle<TextureGraph>,
    seed: u32,
    shape: Shape,
    image: Handle<Image>,
}

struct Reading {
    /// Behind a mutex only because a resource must be `Sync` and a boxed future is not.
    readback: Mutex<Readback>,
    shape: Shape,
    image: Handle<Image>,
}

/// Bakes waiting on their graph or the device, and bakes waiting on the GPU.
#[derive(Resource, Default)]
pub struct Bakes {
    waiting: Vec<Request>,
    reading: Vec<Reading>,
    /// Built once: its pipelines are most of what a first bake costs.
    baker: Option<(Baker, RenderDevice)>,
}

impl Bakes {
    /// Bake `graph`'s output layer into the image behind `image`, with `seed` as the graph's
    /// global seed.
    pub fn request(
        &mut self,
        graph: Handle<TextureGraph>,
        seed: u32,
        shape: Shape,
        image: Handle<Image>,
    ) {
        self.waiting.push(Request {
            graph,
            seed,
            shape,
            image,
        });
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
            continue;
        }
        let Some(TextureGraph(graph)) = graphs.get(&request.graph) else {
            bakes.waiting.push(request);
            continue;
        };
        match start(baker, graph, request.seed, request.shape) {
            Ok(readback) => bakes.reading.push(Reading {
                readback: Mutex::new(readback),
                shape: request.shape,
                image: request.image,
            }),
            Err(e) => warn!("a procedural texture did not bake, drawing without it: {e}"),
        }
    }

    if bakes.reading.is_empty() {
        return;
    }
    // Fires the map callbacks on native. A browser fires them from its own event loop and this
    // is a no-op there.
    let _ = device.poll(PollType::Poll);
    let mut landed = false;
    bakes.reading.retain(|reading| {
        let polled = reading
            .readback
            .lock()
            .unwrap()
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()));
        let Poll::Ready(field) = polled else {
            return true;
        };
        debug!("baked a procedural texture: {:?}", reading.shape);
        let _ = images.insert(&reading.image, image_of(reading.shape, field.bytes));
        landed = true;
        false
    });
    if landed {
        // Visiting every material marks it changed, which is what makes Bevy rebuild the bind
        // groups that still hold a placeholder's view.
        for _ in populations.iter_mut() {}
        for _ in surfaces.iter_mut() {}
    }
}

fn start(baker: &mut Baker, graph: &Graph, seed: u32, shape: Shape) -> Result<Readback, String> {
    let layer = graph
        .output
        .color
        .ok_or("the graph's output has no color layer")?;
    let eval = EvalCtx {
        seed,
        ..EvalCtx::default()
    };
    let ctx = baker.ctx().clone();
    let (texture, size) = match shape {
        Shape::Volume(n) => {
            let volume = baker
                .bake_scalar_volume(graph, layer, n, n, FORMAT, &eval)
                .map_err(|e| e.to_string())?;
            (volume.texture, volume.size)
        }
        Shape::Cube(n) => {
            let cube = baker
                .bake_scalar_cube(graph, layer, n, FORMAT, &eval)
                .map_err(|e| e.to_string())?;
            (cube.texture, (n, n, CUBE_FACES))
        }
    };
    Ok(Box::pin(async move {
        read_scalar_volume_async(&ctx, &texture, size, FORMAT).await
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
        let shape = Shape::Volume(GRAIN_TEXELS);
        let image = world
            .resource_mut::<Assets<Image>>()
            .add(placeholder(shape));
        world
            .resource_mut::<Bakes>()
            .request(graph, EvalCtx::default().seed, shape, image.clone());
        Self { image }
    }
}

pub struct ProceduralTexturesPlugin;

impl Plugin for ProceduralTexturesPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<TextureGraph>()
            .init_asset_loader::<TextureGraphLoader>()
            .init_resource::<Bakes>()
            .init_resource::<PopulationGrain>()
            .add_plugins(crate::surfaces::SurfacesPlugin)
            .add_systems(Update, run_bakes);
    }
}

#[cfg(test)]
mod tests {
    use em_render::population_material::GRAIN_TILE;
    use texture_graph_core::{LayerKind, NoiseKernel, NoiseRange};

    use super::*;

    /// What the shader assumes of the shipped grain, which an edit in the editor could break
    /// without anything else noticing: the volume must tile on the unit cube, with as many
    /// grains across it as the shader divides by, centred on half.
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
            "the shader recentres on half itself"
        );
        assert_eq!(noise.frequency, GRAIN_TILE);
        assert_eq!(
            noise.period, [GRAIN_TILE as u32; 3],
            "seamless on the cube is period == frequency"
        );
    }
}
