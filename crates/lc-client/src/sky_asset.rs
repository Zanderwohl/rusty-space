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
use lc_world::sky::galaxy::{GalaxyError, Manifest};
use lc_world::sky::{CatalogStar, StarProvider};

/// A decoded sky chunk.
#[derive(Asset, TypePath, Debug)]
pub struct Sky {
    pub source: String,
    pub stars: Vec<CatalogStar>,
    /// Records the chunk held that did not describe a star.
    pub skipped: usize,
}

impl StarProvider for Sky {
    fn name(&self) -> &str {
        &self.source
    }
    fn stars(&self) -> &[CatalogStar] {
        &self.stars
    }
}

#[derive(Debug)]
pub enum SkyLoadError {
    Io(std::io::Error),
    Chunk(ChunkError),
    Galaxy(GalaxyError),
}

impl std::fmt::Display for SkyLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Chunk(e) => write!(f, "{e}"),
            Self::Galaxy(e) => write!(f, "{e}"),
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

/// An authored galaxy: a manifest, and the chunks it names, fetched beside it.
#[derive(Default, TypePath)]
pub struct GalaxyLoader;

impl AssetLoader for GalaxyLoader {
    type Asset = Sky;
    type Settings = ();
    type Error = SkyLoadError;

    async fn load(&self, reader: &mut dyn Reader, _settings: &(), load: &mut LoadContext<'_>) -> Result<Sky, Self::Error> {
        let mut raw = Vec::new();
        reader.read_to_end(&mut raw).await?;
        let text = String::from_utf8(raw).map_err(|e| SkyLoadError::Io(std::io::Error::other(e)))?;
        let manifest = Manifest::parse(&text).map_err(SkyLoadError::Galaxy)?;
        let mut bytes = Vec::with_capacity(manifest.chunks.len());
        for c in &manifest.chunks {
            let file = c.checked_file().map_err(SkyLoadError::Galaxy)?;
            // Resolved against the manifest's own path, which keeps its source: a manifest the
            // shard named is fetched over `https`, and its chunks must be too.
            let path = load.path().resolve_embed(&bevy::asset::AssetPath::parse(file));
            bytes.push(load.read_asset_bytes(path).await.map_err(|e| SkyLoadError::Io(std::io::Error::other(e.to_string())))?);
        }
        let provider = manifest.load(&bytes).map_err(SkyLoadError::Galaxy)?;
        Ok(Sky { source: provider.name().to_owned(), skipped: provider.skipped, stars: provider.into_stars() })
    }

    fn extensions(&self) -> &[&str] {
        &["galaxy"]
    }
}

pub struct SkyAssetPlugin;

impl Plugin for SkyAssetPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Sky>().init_asset_loader::<SkyLoader>().init_asset_loader::<GalaxyLoader>();
    }
}

#[cfg(test)]
mod tests {
    use bevy::asset::AssetPath;

    /// A chunk named by a manifest on the CDN is fetched from the CDN, beside the manifest.
    #[test]
    fn a_chunk_resolves_beside_its_manifest_and_keeps_its_source() {
        let manifest = AssetPath::parse("https://cdn.example/galaxy/a/0123456789abcdef.galaxy");
        let chunk = manifest.resolve_embed(&AssetPath::parse("chunks/fedcba9876543210.lcg"));
        assert_eq!(chunk, AssetPath::parse("https://cdn.example/galaxy/a/chunks/fedcba9876543210.lcg"));
    }
}
