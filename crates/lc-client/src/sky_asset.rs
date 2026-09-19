//! The sky as a Bevy asset.
//!
//! Going through the asset server rather than reading a file is what makes the browser build
//! and the desktop build the same code: on native the reader is a file, in a browser it is a
//! fetch, and neither is visible from here. It is also the only way to load anything in a
//! browser, where there is no filesystem and no blocking IO.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use lc_world::sky::chunk::{ChunkError, ChunkProvider};
use lc_world::sky::{CatalogueStar, StarProvider};

/// A decoded sky chunk.
#[derive(Asset, TypePath, Debug)]
pub struct Sky {
    pub source: String,
    pub stars: Vec<CatalogueStar>,
    /// Records the chunk held that did not describe a star.
    pub skipped: usize,
}

impl StarProvider for Sky {
    fn name(&self) -> &str {
        &self.source
    }
    fn stars(&self) -> &[CatalogueStar] {
        &self.stars
    }
}

#[derive(Debug)]
pub enum SkyLoadError {
    Io(std::io::Error),
    Chunk(ChunkError),
}

impl std::fmt::Display for SkyLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Chunk(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for SkyLoadError {}

impl From<std::io::Error> for SkyLoadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Default, TypePath)]
pub struct SkyLoader;

impl AssetLoader for SkyLoader {
    type Asset = Sky;
    type Settings = ();
    type Error = SkyLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load: &mut LoadContext<'_>,
    ) -> Result<Sky, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let provider = ChunkProvider::decode(&bytes).map_err(SkyLoadError::Chunk)?;
        Ok(Sky {
            source: provider.name().to_owned(),
            skipped: provider.skipped,
            stars: provider.stars().to_vec(),
        })
    }

    fn extensions(&self) -> &[&str] {
        &["lcsky"]
    }
}

pub struct SkyAssetPlugin;

impl Plugin for SkyAssetPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Sky>().init_asset_loader::<SkyLoader>();
    }
}
