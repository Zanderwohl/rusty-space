//! Procedural textures: graphs authored in texture-graph, baked on the GPU at load.
//!
//! A graph is an asset like any other, so in a browser it arrives from the CDN beside the
//! shaders. The bake runs on Bevy's own device, then reads back into an ordinary [`Image`] —
//! Bevy 0.19 has no way to adopt a foreign `wgpu::Texture` as an image, and a single-channel
//! volume is small enough that the round trip costs nothing worth avoiding.
//!
//! Until a bake lands, its image holds a flat placeholder, so the material that binds it draws
//! unchanged but for the texture. See `lightcone/docs/07-rendering.md`.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, PollType, TextureDimension, TextureFormat};
use bevy::render::renderer::{RenderAdapter, RenderDevice, RenderQueue};
use em_render::population_material::PopulationMaterial;
use texture_graph_core::{EvalCtx, Graph, LoadError, load_from_str};
use texture_graph_gpu::{Baker, DeviceCtx, ScalarFormat, ScalarImage, read_scalar_volume_async};

/// Where the population grain graph lives under the asset root.
const POPULATION_GRAIN: &str = "textures/population_grain.tgraph";

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
        reader.read_to_end(&mut bytes).await.map_err(TextureGraphLoadError::Io)?;
        let text = String::from_utf8(bytes).map_err(TextureGraphLoadError::Utf8)?;
        let file = load_from_str(&text).map_err(TextureGraphLoadError::Graph)?;
        Ok(TextureGraph(file.graph))
    }

    fn extensions(&self) -> &[&str] {
        &["tgraph"]
    }
}

/// The grain every population and ring samples. The handle is stable: the bake replaces the
/// image behind it rather than issuing a new one, so materials built before it lands pick it up.
#[derive(Resource)]
pub struct PopulationGrain {
    pub image: Handle<Image>,
    graph: Handle<TextureGraph>,
    bake: Bake,
}

type Readback = Pin<Box<dyn Future<Output = ScalarImage> + Send>>;

enum Bake {
    Waiting,
    /// Behind a mutex only because a resource must be `Sync` and a boxed future is not.
    Reading { readback: Mutex<Readback>, device: RenderDevice },
    Done,
}

impl FromWorld for PopulationGrain {
    fn from_world(world: &mut World) -> Self {
        let graph = world.resource::<AssetServer>().load(POPULATION_GRAIN);
        let image = world.resource_mut::<Assets<Image>>().add(flat_grain());
        Self { image, graph, bake: Bake::Waiting }
    }
}

/// Half-way, which is no grain at all: the shader centres the field on it.
fn flat_grain() -> Image {
    grain_image(1, vec![128])
}

fn grain_image(edge: u32, bytes: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d { width: edge, height: edge, depth_or_array_layers: edge },
        TextureDimension::D3,
        bytes,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// Start the bake once the graph and the device both exist, then finish it once the GPU has.
///
/// Neither is there at startup: the graph is a fetch in a browser, and Bevy may still be
/// bringing its renderer up. A failed bake leaves the placeholder, which is a texture missing
/// rather than a game that cannot start.
fn bake_population_grain(
    mut grain: ResMut<PopulationGrain>,
    graphs: Res<Assets<TextureGraph>>,
    device: Option<Res<RenderDevice>>,
    queue: Option<Res<RenderQueue>>,
    adapter: Option<Res<RenderAdapter>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<PopulationMaterial>>,
) {
    let grain = &mut *grain;
    match &grain.bake {
        Bake::Done => {}
        Bake::Waiting => {
            let (Some(TextureGraph(graph)), Some(device), Some(queue), Some(adapter)) =
                (graphs.get(&grain.graph), device, queue, adapter)
            else {
                return;
            };
            let ctx = DeviceCtx::from_shared(
                Arc::new((**adapter.0).clone()),
                Arc::new(device.wgpu_device().clone()),
                Arc::new((**queue.0).clone()),
            );
            grain.bake = match start(graph, ctx) {
                Ok(readback) => {
                    Bake::Reading { readback: Mutex::new(readback), device: device.clone() }
                }
                Err(e) => {
                    warn!("population grain did not bake, drawing without it: {e}");
                    Bake::Done
                }
            };
        }
        Bake::Reading { readback, device } => {
            // Fires the map callback on native. A browser fires it from its own event loop and
            // this is a no-op there.
            let _ = device.poll(PollType::Poll);
            let polled =
                readback.lock().unwrap().as_mut().poll(&mut Context::from_waker(Waker::noop()));
            let Poll::Ready(field) = polled else { return };
            let _ = images.insert(&grain.image, grain_image(field.width, field.bytes));
            // Visiting every material marks it changed, which is what makes Bevy rebuild the
            // bind groups that still hold the placeholder's view.
            for _ in materials.iter_mut() {}
            info!("population grain baked at {0}x{0}x{0}", field.width);
            grain.bake = Bake::Done;
        }
    }
}

fn start(graph: &Graph, ctx: DeviceCtx) -> Result<Readback, String> {
    let layer = graph.output.color.ok_or("the graph's output has no color layer")?;
    let format = ScalarFormat::R8Unorm;
    let volume = Baker::new(ctx.clone())
        .bake_scalar_volume(graph, layer, GRAIN_TEXELS, GRAIN_TEXELS, format, &EvalCtx::default())
        .map_err(|e| e.to_string())?;
    Ok(Box::pin(async move {
        read_scalar_volume_async(&ctx, &volume.texture, volume.size, volume.format).await
    }))
}

pub struct ProceduralTexturesPlugin;

impl Plugin for ProceduralTexturesPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<TextureGraph>()
            .init_asset_loader::<TextureGraphLoader>()
            .init_resource::<PopulationGrain>()
            .add_systems(Update, bake_population_grain);
    }
}
