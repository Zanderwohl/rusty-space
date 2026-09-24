//! A hull drawn in regions, each a material from a palette the caller supplies. The host
//! supplies `shaders/hull.wgsl`. See `lightcone/docs/32-ship-rendering.md`.
//!
//! A distance-field mesh has no UVs, so each region's tile is mapped triplanar from the mesh's
//! own position, which is the ship's frame in meters. The tile is [`HullUniform::detail`]'s
//! meters across whatever the hull's size, which is what makes a big hull look big. Tiles are
//! mipmapped, so detail smaller than a pixel is drawn as its own average instead of shimmering.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension, VertexFormat,
};
use bevy::shader::ShaderRef;
use bevy_mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef, VertexAttributeValues};

/// Each vertex's weight for every region of the palette, a byte apiece, four regions an
/// attribute. A mesher writes its nearest parts' blend through [`region_weights`].
///
/// Weights rather than a pair of indices and a share: a triangle whose corners name different
/// pairs cannot interpolate a share measured against each corner's own pair, and every
/// triangle crossing the edge of a fillet is one.
pub const ATTRIBUTE_HULL_REGIONS: [MeshVertexAttribute; 4] = [
    MeshVertexAttribute::new("HullRegions0", 0x4855_4C4C_0000_0001, VertexFormat::Unorm8x4),
    MeshVertexAttribute::new("HullRegions1", 0x4855_4C4C_0000_0002, VertexFormat::Unorm8x4),
    MeshVertexAttribute::new("HullRegions2", 0x4855_4C4C_0000_0003, VertexFormat::Unorm8x4),
    MeshVertexAttribute::new("HullRegions3", 0x4855_4C4C_0000_0004, VertexFormat::Unorm8x4),
];

/// `(across, along)`: signed meters from the vertex to the nearest seam between two regions,
/// positive on the lower-numbered region's side, and meters along that seam from anywhere fixed
/// on it. What a bolted region draws its row of heads by. A vertex far from every seam writes a
/// large `across`.
///
/// The side comes from the sign rather than from which region weighs more, because in the
/// middle of a fillet the two weights differ by less than a byte.
pub const ATTRIBUTE_HULL_SEAM: MeshVertexAttribute =
    MeshVertexAttribute::new("HullSeam", 0x4855_4C4C_0000_0005, VertexFormat::Float32x2);

/// Regions a palette may hold. Must match `REGIONS` in `hull.wgsl`.
pub const REGIONS: usize = 16;

/// Anything at or past this in [`HullUniform::reveal`]'s `w` is plated everywhere.
pub const ALL_PLATED: f32 = 1.0e30;

/// [`ATTRIBUTE_HULL_REGIONS`] for a point between `nearest` and `second`, taking `share` of
/// `second`. The shader draws the two heaviest regions at a point, so a triple junction
/// drops its lightest.
pub fn region_weights(nearest: u32, second: u32, share: f32) -> [[u8; 4]; 4] {
    assert!(
        (nearest as usize) < REGIONS && (second as usize) < REGIONS,
        "regions {nearest} and {second}: a palette holds {REGIONS}"
    );
    let share = share.clamp(0.0, 1.0);
    let mut weights = [0.0f32; REGIONS];
    weights[nearest as usize] += 1.0 - share;
    weights[second as usize] += share;
    std::array::from_fn(|a| std::array::from_fn(|c| (weights[4 * a + c] * 255.0).round() as u8))
}

/// Set [`ATTRIBUTE_HULL_REGIONS`] from one [`region_weights`] a vertex.
pub fn insert_region_weights(mesh: &mut Mesh, weights: &[[[u8; 4]; 4]]) {
    for (a, attribute) in ATTRIBUTE_HULL_REGIONS.iter().enumerate() {
        let values: Vec<[u8; 4]> = weights.iter().map(|w| w[a]).collect();
        mesh.insert_attribute(*attribute, VertexAttributeValues::Unorm8x4(values));
    }
}

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct HullUniform {
    /// World direction to the star; `w` is the light on the unlit side, as a fraction.
    pub to_star: Vec4,
    /// Starlight a white surface facing the star sends, as linear display light before the
    /// tone map.
    pub reflected: Vec4,
    /// `(surface_reference, stops, 0, 0)`, evaluated per fragment as `body_surface.wgsl` does:
    /// lights and sunlight mix across the hull and the curve is logarithmic.
    pub exposure: Vec4,
    /// `(tile_m, 0, 0, 0)`: meters one tile spans.
    pub detail: Vec4,
    /// Plating's sweep: `(origin, front)`, ship frame, meters. A panel is plated once `front`
    /// passes its distance from `origin` plus its hashed share of the spread.
    pub reveal: Vec4,
    /// `(panel_m, spread_m, 0, 0)`.
    pub reveal_panel: Vec4,
    /// A bolted region's row of heads along each seam: `(pitch_m, head_radius_m, offset_m,
    /// albedo)`, the offset measured from the seam into the bolted region.
    pub bolts: Vec4,
    /// Which regions are bolted, one bit a region.
    pub bolted: u32,
    /// Per region, what a fully lit texel of its lights sends, in [`Self::reflected`]'s units.
    /// A real, small power: it shows on a night side and is lost against a lit one.
    pub emitted: [Vec4; REGIONS],
}

impl Default for HullUniform {
    fn default() -> Self {
        Self {
            to_star: Vec4::new(0.0, 0.0, 1.0, 0.0),
            reflected: Vec4::ONE,
            exposure: Vec4::new(1.0, 5.0, 0.0, 0.0),
            detail: Vec4::new(64.0, 0.0, 0.0, 0.0),
            reveal: Vec4::new(0.0, 0.0, 0.0, ALL_PLATED),
            reveal_panel: Vec4::new(8.0, 0.0, 0.0, 0.0),
            bolts: Vec4::new(1.5, 0.25, 0.8, 0.45),
            bolted: 0,
            emitted: [Vec4::ZERO; REGIONS],
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct HullMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: HullUniform,
    /// One layer a region, from [`tile_array`] with [`Tile::Albedo`].
    #[texture(1, dimension = "2d_array", visibility(fragment))]
    #[sampler(2, visibility(fragment))]
    pub albedo: Handle<Image>,
    /// The same with [`Tile::Lights`]: the share of each texel that is lit.
    #[texture(3, dimension = "2d_array", visibility(fragment))]
    pub lights: Handle<Image>,
}

impl Material for HullMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/hull.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/hull.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            ATTRIBUTE_HULL_REGIONS[0].at_shader_location(2),
            ATTRIBUTE_HULL_REGIONS[1].at_shader_location(3),
            ATTRIBUTE_HULL_REGIONS[2].at_shader_location(4),
            ATTRIBUTE_HULL_REGIONS[3].at_shader_location(5),
            ATTRIBUTE_HULL_SEAM.at_shader_location(6),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

/// What a tile holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tile {
    /// sRGB with straight alpha, four bytes a texel; averaged in linear light.
    Albedo,
    /// One byte a texel, linear.
    Lights,
}

impl Tile {
    fn bytes(self) -> usize {
        match self {
            Tile::Albedo => 4,
            Tile::Lights => 1,
        }
    }
}

/// Square tiles of `texels` a side, one a region, as a repeating mipmapped array.
///
/// Mipmapped on the CPU because the bake arrives without levels, and a level is the average
/// of the four texels under it, which is exactly the fade to average that stops a shimmer.
/// `texels` is a power of two.
pub fn tile_array(tiles: &[Vec<u8>], texels: u32, tile: Tile) -> Image {
    assert!(texels.is_power_of_two(), "a tile of {texels} texels has no whole mip chain");
    let levels = texels.trailing_zeros() + 1;
    let mut data = Vec::new();
    for texels_of in tiles {
        assert_eq!(texels_of.len(), (texels * texels) as usize * tile.bytes());
        let mut level = texels_of.clone();
        let mut side = texels as usize;
        loop {
            data.extend_from_slice(&level);
            if side == 1 {
                break;
            }
            level = halved(&level, side, tile);
            side /= 2;
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: texels,
            height: texels,
            depth_or_array_layers: tiles.len() as u32,
        },
        TextureDimension::D2,
        // `Image::new` checks the length against the first level alone.
        vec![0; (texels * texels) as usize * tile.bytes() * tiles.len()],
        match tile {
            Tile::Albedo => TextureFormat::Rgba8UnormSrgb,
            Tile::Lights => TextureFormat::R8Unorm,
        },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        // A hull is mostly seen obliquely, where a trilinear footprint blurs panels to gray
        // long before they are below a pixel.
        anisotropy_clamp: 16,
        ..default()
    });
    // Explicit: with one layer Bevy would infer a plain 2D view.
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image
}

/// The next level down: each texel the mean of the four above it.
fn halved(level: &[u8], side: usize, tile: Tile) -> Vec<u8> {
    let bytes = tile.bytes();
    let half = side / 2;
    let mut out = vec![0u8; half * half * bytes];
    for y in 0..half {
        for x in 0..half {
            for c in 0..bytes {
                let at = |dx: usize, dy: usize| level[((2 * y + dy) * side + 2 * x + dx) * bytes + c];
                let four = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
                let srgb = tile == Tile::Albedo && c < 3;
                let mean = if srgb {
                    to_srgb(four.iter().map(|&v| to_linear(v)).sum::<f32>() / 4.0)
                } else {
                    four.iter().map(|&v| v as f32).sum::<f32>() / 4.0
                };
                out[(y * half + x) * bytes + c] = mean.round() as u8;
            }
        }
    }
    out
}

fn to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Back to an sRGB byte, unrounded.
fn to_srgb(linear: f32) -> f32 {
    let c = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    c.clamp(0.0, 1.0) * 255.0
}

pub struct HullMaterialPlugin;

impl Plugin for HullMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<HullMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fillets_weights_land_on_its_two_regions() {
        let w = region_weights(5, 2, 0.25);
        assert_eq!(w[1][1], 191);
        assert_eq!(w[0][2], 64);
        assert_eq!(w.iter().flatten().map(|&b| b as u32).sum::<u32>(), 255);
        assert_eq!(region_weights(9, 9, 0.4)[2][1], 255);
    }

    #[test]
    #[should_panic(expected = "a palette holds 16")]
    fn a_region_past_the_palette_is_refused() {
        region_weights(3, REGIONS as u32, 0.5);
    }

    /// The last level is the whole tile's average, in linear light for albedo: averaging the
    /// sRGB bytes would draw a far checker of black and white at 128, a quarter too dark.
    #[test]
    fn the_smallest_level_is_the_tiles_linear_mean() {
        let texels = 4u32;
        let mut albedo = Vec::new();
        let mut lights = Vec::new();
        for i in 0..texels * texels {
            let on = (i + i / texels) % 2 == 0;
            let v = if on { 255 } else { 0 };
            albedo.extend_from_slice(&[v, v, v, 255]);
            lights.push(v);
        }
        let image = tile_array(&[albedo.clone(), albedo], texels, Tile::Albedo);
        assert_eq!(image.texture_descriptor.mip_level_count, 3);
        let data = image.data.as_ref().unwrap();
        // 16 + 4 + 1 texels a layer.
        let per_layer = 21 * 4;
        assert_eq!(data.len(), 2 * per_layer);
        let last = &data[per_layer - 4..per_layer];
        assert_eq!(last[0], to_srgb(0.5).round() as u8);
        assert_eq!(last[0], 188);
        assert_eq!(last[3], 255);

        let image = tile_array(&[lights], texels, Tile::Lights);
        assert_eq!(*image.data.as_ref().unwrap().last().unwrap(), 128);
    }
}
