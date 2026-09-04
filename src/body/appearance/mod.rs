use std::collections::HashMap;
use bevy::asset::RenderAssetUsages;
use bevy::color::LinearRgba;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Appearance *data* lives in `em-sim`; this module holds the Bevy-side rendering of it.
pub use em_sim::appearance::{Appearance, AppearanceColor, DebugBall, StarBall};

/// Turns appearance data into Bevy render components.
///
/// A trait rather than inherent methods because the data types now live in `em-sim`,
/// and Rust does not allow inherent impls on foreign types. The associated output lets
/// a star also contribute a `PointLight`.
pub trait PbrBundle {
    type Output;
    fn pbr_bundle(
        &self,
        cache: &mut ResMut<AssetCache>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut ResMut<Assets<Image>>,
    ) -> Self::Output;
}

#[derive(Resource, Default)]
pub struct AssetCache {
    pub meshes: HashMap<String, Handle<Mesh>>,
    pub materials: HashMap<String, Handle<StandardMaterial>>,
}

impl PbrBundle for DebugBall {
    type Output = (Mesh3d, MeshMaterial3d<StandardMaterial>);
    fn pbr_bundle(&self,
                      cache: &mut ResMut<AssetCache>,
                      meshes: &mut Assets<Mesh>,
                      materials: &mut Assets<StandardMaterial>,
                      images: &mut ResMut<Assets<Image>>,
    ) -> Self::Output {
        let color = Color::srgb(self.color.r as f32 / 255.0, self.color.g as f32 / 255.0, self.color.b as f32 / 255.0);
        let mesh_key = format!("icosphere_{}", self.radius);
        let material_key = format!("color_{:02x}{:02x}{:02x}", self.color.r, self.color.g, self.color.b);

        let mesh_handle = cache.meshes.entry(mesh_key.clone()).or_insert_with(|| {
            meshes.add(Sphere::new(1.0f32).mesh().ico(5).unwrap())
        }).clone();

        let material_handle = cache.materials.entry(material_key.clone()).or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: color,
                metallic: 0.1,
                perceptual_roughness: 1.0,
                ..Default::default()
            })
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

impl PbrBundle for StarBall {
    type Output = (Mesh3d, MeshMaterial3d<StandardMaterial>, PointLight);
    fn pbr_bundle(&self,
                      cache: &mut ResMut<AssetCache>,
                      meshes: &mut Assets<Mesh>,
                      materials: &mut Assets<StandardMaterial>,
                      images: &mut ResMut<Assets<Image>>,
    ) -> Self::Output {
        let color = Color::srgb(self.color.r as f32 / 255.0, self.color.g as f32 / 255.0, self.color.b as f32 / 255.0);
        let mesh_key = format!("icosphere_{}", self.radius);
        let material_key = format!("color_{:02x}{:02x}{:02x}_{:03x}:{:03x}:{:03x}", self.color.r, self.color.g, self.color.b, self.light.r, self.light.g, self.light.b);

        let mesh_handle = cache.meshes.entry(mesh_key.clone()).or_insert_with(|| {
            meshes.add(Sphere::new(1.0f32).mesh().ico(5).unwrap())
        }).clone();

        let material_handle = cache.materials.entry(material_key.clone()).or_insert_with(|| {
            let e = self.emissive_luminance();
            materials.add(StandardMaterial {
                base_color: color,
                emissive: LinearRgba::rgb(
                    (self.light.r as f32 / 255.0) * e,
                    (self.light.g as f32 / 255.0) * e,
                    (self.light.b as f32 / 255.0) * e,
                ),
                ..Default::default()
            })
        }).clone();

        let light_color = Color::srgb(self.light.r as f32 / 255.0, self.light.g as f32 / 255.0, self.light.b as f32 / 255.0);
        // Initialize intensity for the default scale (1e-9). It will be updated dynamically.
        let light = PointLight {
            color: light_color,
            intensity: self.intensity() * (1e-9f32 * 1e-9f32),
            range: 1e14 * 1e-9,
            radius: 0.1,
            shadows_enabled: true,
            ..Default::default()
        };

        (
            Mesh3d(mesh_handle),
            MeshMaterial3d(material_handle),
            light
        )
    }
}