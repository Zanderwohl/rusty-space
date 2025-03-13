use bevy::asset::RenderAssetUsages;
use bevy::prelude::{default, Assets, Color, Handle, Image, Mesh, Mesh3d, MeshMaterial3d, Meshable, PbrBundle, ResMut, Resource, Sphere, StandardMaterial, Transform, Vec3};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::utils::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Resource, Default)]
pub struct AssetCache {
    pub meshes: HashMap<String, Handle<Mesh>>,
    pub materials: HashMap<String, Handle<StandardMaterial>>,
}

#[derive(Serialize, Deserialize, Default)]
pub enum Appearance {
    #[default]
    Empty,
    DebugBall(DebugBall),
}

impl Appearance {
    pub fn pbr_bundle(&self,
                      cache: &mut ResMut<AssetCache>,
                      meshes: &mut Assets<Mesh>,
                      materials: &mut Assets<StandardMaterial>,
                      mut images: ResMut<Assets<Image>>,
    ) -> (Mesh3d, MeshMaterial3d<StandardMaterial>) {
        match self {
            Appearance::Empty => self.empty(cache, meshes, materials, images),
            Appearance::DebugBall(debug_ball) => debug_ball.pbr_bundle(cache, meshes, materials, images),
        }
    }

    pub fn empty(&self,
                 cache: &mut ResMut<AssetCache>,
                 meshes: &mut Assets<Mesh>,
                 materials: &mut Assets<StandardMaterial>,
                 mut images: ResMut<Assets<Image>>,
    ) -> (Mesh3d, MeshMaterial3d<StandardMaterial>) {
        let material_handle = cache.materials.entry("debug_uv".into()).or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color_texture: Some(images.add(uv_debug_texture())),
                ..default()
            })
        }).clone();

        let mesh_handle = cache.meshes.entry("debug_ico_1".into()).or_insert_with(|| {
            meshes.add(Sphere::default().mesh().ico(5).unwrap())
        }).clone();

        (
            Mesh3d(mesh_handle),
            MeshMaterial3d(material_handle),
        )
    }
}

#[derive(Serialize, Deserialize)]
pub struct DebugBall {
    pub radius: f64,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl DebugBall {
    pub fn pbr_bundle(&self,
                      cache: &mut ResMut<AssetCache>,
                      meshes: &mut Assets<Mesh>,
                      materials: &mut Assets<StandardMaterial>,
                      mut images: ResMut<Assets<Image>>,
    ) -> (Mesh3d, MeshMaterial3d<StandardMaterial>) {
        let color = Color::srgb_u8(self.r, self.g, self.b);
        let mesh_key = format!("icosphere_{}", self.radius);
        let material_key = format!("color_{:02x}{:02x}{:02x}", self.r, self.g, self.b);

        let mesh_handle = cache.meshes.entry(mesh_key.clone()).or_insert_with(|| {
            meshes.add(Sphere::default().mesh().ico(5).unwrap())
        }).clone();

        let material_handle = cache.materials.entry(material_key.clone()).or_insert_with(|| {
            materials.add(StandardMaterial { base_color: color, ..Default::default() })
        }).clone();

        (
            Mesh3d(mesh_handle),
            MeshMaterial3d(material_handle),
        )
    }
}

fn uv_debug_texture() -> Image {
    const TEXTURE_SIZE: usize = 8;

    let mut palette: [u8; 32] = [
        255, 102, 159, 255, 255, 159, 102, 255, 236, 255, 102, 255, 121, 255, 102, 255, 102, 255,
        198, 255, 102, 198, 255, 255, 121, 102, 255, 255, 236, 102, 255, 255,
    ];

    let mut texture_data = [0; TEXTURE_SIZE * TEXTURE_SIZE * 4];
    for y in 0..TEXTURE_SIZE {
        let offset = TEXTURE_SIZE * y * 4;
        texture_data[offset..(offset + TEXTURE_SIZE * 4)].copy_from_slice(&palette);
        palette.rotate_right(4);
    }

    Image::new_fill(
        Extent3d {
            width: TEXTURE_SIZE as u32,
            height: TEXTURE_SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &texture_data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

