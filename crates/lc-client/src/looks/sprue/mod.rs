//! A layered sprue: an opaque plated base under two cutout layers of mazed pipe, each flat and
//! normal-mapped, standing off one another and lit from the gaps by sodium lamps baked into the
//! tiles. A box wears it as three nested boxes, one a layer; see [`Sprue::lift`].

pub mod bake;
pub mod tiles;

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

use bake::Lamps;
use tiles::{Maze, Rng, Surface};

pub const TILE_M: f32 = 24.0;

/// 4.7 cm a texel. Sixteen variants of three layers at 1024 would take a quarter of a gigabyte.
pub const TILE_TEXELS: u32 = 512;

/// Two choices a tile edge: a variant is left + 2 right + 4 bottom + 8 top, as sprue.wgsl reads it.
pub const VARIANTS: usize = 16;

/// Meters over the base: layer 1, layer 2, the deep lamps, and the lamps riding layer 1.
pub const HEIGHTS: Vec4 = Vec4::new(0.9, 1.9, 0.4, 1.25);

/// High-pressure sodium, linear.
pub const SODIUM: Vec3 = Vec3::new(1.0, 0.72, 0.2);

const CONDUIT: Maze = Maze {
    cells: 8,
    radii: &[0.3, 0.4, 0.5],
    prune: 0.5,
    prune_passes: 2,
    doubled: 0.3,
    boxed: 0.5,
    collar_m: 1.6,
    crossing: 0.2,
};

const CAPILLARY: Maze = Maze {
    cells: 16,
    radii: &[0.1, 0.14, 0.18],
    prune: 0.6,
    prune_passes: 3,
    doubled: 0.25,
    boxed: 0.35,
    collar_m: 0.7,
    crossing: 0.18,
};

pub struct Sprue {
    pub sodium: Vec3,
    pub to_star: Vec3,
    pub star: f32,
    /// The star's turn about world Y, rad/s; see [`super::spin`].
    pub star_spin: f32,
    /// The lamps' power, the haze they light, and their bulbs'.
    pub lamp: f32,
    pub scatter: f32,
    pub bulb: f32,
    /// Of 4 m cells with a lamp over the base, and riding layer 1.
    pub deep: f32,
    pub riding: f32,
    /// How far a lamp's light goes. Twice it, and the tile less twice it, are whole 4 m cells.
    pub reach_m: f32,
    /// The slice of each lamp's circle it lights, aimed at random; 360 is a bare bulb.
    pub beam_deg: f32,
    pub exposure_stops: f32,
    pub seed: u64,
}

impl Default for Sprue {
    fn default() -> Self {
        Self {
            sodium: SODIUM,
            to_star: super::direction(25.0, -60.0),
            star: 0.2,
            star_spin: 0.0,
            lamp: 1.0,
            scatter: 1.0,
            bulb: 1.0,
            deep: 0.35,
            riding: 0.3,
            reach_m: 2.0,
            beam_deg: 70.0,
            exposure_stops: 0.0,
            seed: 7,
        }
    }
}

/// Every variant's layers, and the lamp light baked into each.
pub struct Baked {
    /// `[variant][layer]`.
    pub shaped: Vec<Vec<Surface>>,
    /// `[variant * 3 + layer]`, in units of the lamps' color.
    pub light: Vec<Vec<Vec3>>,
    pub lamps: usize,
    /// The share of texels where layer 1 could carry a riding lamp.
    pub carried: f32,
}

impl Sprue {
    /// Meters each layer's box stands off the base's on every side.
    pub fn lift(layer: usize) -> f32 {
        [0.0, HEIGHTS.x, HEIGHTS.y][layer]
    }

    /// Seconds of work, spread over a thread a variant.
    pub fn bake(&self) -> Baked {
        let texels = TILE_TEXELS as usize;
        let mut rng = Rng::new(self.seed);
        let conduit = tiles::MazeEdges::new(&CONDUIT, &mut rng);
        let capillary = tiles::MazeEdges::new(&CAPILLARY, &mut rng);
        let mut spec = Lamps {
            cell_m: 4.0,
            reach_m: self.reach_m,
            beam_rad: self.beam_deg.to_radians(),
            deep_share: self.deep,
            riding_share: self.riding,
            carried: 0.0,
            heights: HEIGHTS,
            scatter: 0.004 * self.scatter,
            bulb_radius_m: 0.09,
            bulb_power: 18.0 * self.bulb,
        };
        let seed = self.seed;
        let (conduit, capillary) = (&conduit, &capillary);
        let shape = |v: usize| {
            let edges = [v & 1, v >> 1 & 1, v >> 2 & 1, v >> 3 & 1];
            let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9).wrapping_add(v as u64));
            vec![
                tiles::base(texels, TILE_M, &mut rng).surface(true),
                tiles::maze(texels, TILE_M, &CONDUIT, conduit, edges, &mut rng).surface(false),
                tiles::maze(texels, TILE_M, &CAPILLARY, capillary, edges, &mut rng).surface(false),
            ]
        };
        let shaped: Vec<Vec<Surface>> = std::thread::scope(|scope| {
            let making: Vec<_> = (0..VARIANTS).map(|v| scope.spawn(move || shape(v))).collect();
            making.into_iter().map(|m| m.join().expect("a variant panicked")).collect()
        });
        // Every variant's, since a strip's lamps are its edge's and must not depend on the tile.
        spec.carried = bake::carried(&shaped.iter().map(|layers| &layers[1]).collect::<Vec<_>>());
        let spec = &spec;
        let interiors = bake::interiors(spec, TILE_M, seed);
        let lit: Vec<(Vec<Vec<Vec3>>, usize)> = std::thread::scope(|scope| {
            let baking: Vec<_> = shaped
                .iter()
                .enumerate()
                .zip(&interiors)
                .map(|((v, layers), interior)| {
                    scope.spawn(move || {
                        let lamps = bake::of_variant(spec, layers, seed, v, interior);
                        (bake::bake(spec, layers, &lamps), lamps.len())
                    })
                })
                .collect();
            baking.into_iter().map(|b| b.join().expect("a bake panicked")).collect()
        });
        let lamps = lit.iter().map(|l| l.1).sum();
        let light = lit.into_iter().flat_map(|l| l.0).collect();
        Baked { shaped, light, lamps, carried: spec.carried }
    }

    /// The tile array and the baked light array, every variant's three layers in order.
    pub fn arrays(baked: &Baked) -> (Image, Image) {
        let layers: Vec<&Surface> = baked.shaped.iter().flatten().collect();
        let geometry: Vec<Vec<u8>> = layers.iter().map(|l| l.rgba8()).collect();
        (tiles::tile_array(&geometry, TILE_TEXELS), bake::light_array(&baked.light, &layers))
    }

    /// One material a layer, sharing the tile and light arrays.
    pub fn materials(&self, baked: &Baked, images: &mut Assets<Image>) -> [SprueMaterial; 3] {
        let (tiles, light) = Self::arrays(baked);
        let (tiles, light) = (images.add(tiles), images.add(light));
        std::array::from_fn(|k| SprueMaterial {
            uniforms: SprueUniform {
                layer: Vec4::new(k as f32, TILE_M, 0.0, 0.0),
                heights: HEIGHTS,
                lamp_color: (self.sodium * 6.0 * self.lamp).extend(0.0),
                to_star: self.to_star.extend(self.star_spin),
                star_color: Vec3::splat(self.star).extend(0.0),
                ambient: (self.sodium * 0.015).extend(0.0),
                finish: Vec4::new(2f32.powf(self.exposure_stops), 24.0, 0.35, 0.0),
            },
            tiles: tiles.clone(),
            light: light.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct SprueUniform {
    layer: Vec4,
    heights: Vec4,
    lamp_color: Vec4,
    to_star: Vec4,
    star_color: Vec4,
    ambient: Vec4,
    finish: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct SprueMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    uniforms: SprueUniform,
    #[texture(1, dimension = "2d_array", visibility(fragment))]
    #[sampler(2, visibility(fragment))]
    tiles: Handle<Image>,
    #[texture(3, dimension = "2d_array", visibility(fragment))]
    light: Handle<Image>,
}

impl Material for SprueMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/sprue.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/sprue.wgsl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        super::position_and_normal(descriptor, layout)
    }
}
