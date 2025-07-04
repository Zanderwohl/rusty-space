use std::collections::HashMap;
use bevy::asset::RenderAssetUsages;
use bevy::color::LinearRgba;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use serde::{Deserialize, Serialize};

#[derive(Resource, Default)]
pub struct AssetCache {
    pub meshes: HashMap<String, Handle<Mesh>>,
    pub materials: HashMap<String, Handle<StandardMaterial>>,
}

#[derive(Serialize, Deserialize, Default, Component, Clone)]
pub enum Appearance {
    #[default]
    Empty,
    DebugBall(DebugBall),
    Star(StarBall),
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct AppearanceColor {
    pub r: u16,
    pub g: u16,
    pub b: u16,
}

impl Appearance {
    pub fn radius(&self) -> f64 {
        match self {
            Appearance::Empty => 1.0,
            Appearance::DebugBall(DebugBall { radius, .. }) => *radius,
            Appearance::Star(StarBall { radius, ..}) => *radius,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DebugBall {
    pub radius: f64,
    pub color: AppearanceColor,
}

impl DebugBall {
    pub fn pbr_bundle(&self,
                      cache: &mut ResMut<AssetCache>,
                      meshes: &mut Assets<Mesh>,
                      materials: &mut Assets<StandardMaterial>,
                      mut images: &mut ResMut<Assets<Image>>,
    ) -> (Mesh3d, MeshMaterial3d<StandardMaterial>) {
        let color = Color::srgb(self.color.r as f32 / 255.0, self.color.g as f32 / 255.0, self.color.b as f32 / 255.0);
        let mesh_key = format!("icosphere_{}", self.radius);
        let material_key = format!("color_{:02x}{:02x}{:02x}", self.color.r, self.color.g, self.color.b);

        let mesh_handle = cache.meshes.entry(mesh_key.clone()).or_insert_with(|| {
            meshes.add(Sphere::new(1.0f32).mesh().ico(5).unwrap())
        }).clone();

        let material_handle = cache.materials.entry(material_key.clone()).or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: color,
                metallic: 0.0,
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

#[derive(Serialize, Deserialize, Clone)]
pub struct StarBall {
    pub radius: f64,
    pub color: AppearanceColor,
    pub light: AppearanceColor,
    pub intensity: f32,
}

impl StarBall {
    pub fn pbr_bundle(&self,
                      cache: &mut ResMut<AssetCache>,
                      meshes: &mut Assets<Mesh>,
                      materials: &mut Assets<StandardMaterial>,
                      mut images: &mut ResMut<Assets<Image>>,
    ) -> (Mesh3d, MeshMaterial3d<StandardMaterial>, PointLight) {
        let color = Color::srgb(self.color.r as f32 / 255.0, self.color.g as f32 / 255.0, self.color.b as f32 / 255.0);
        let mesh_key = format!("icosphere_{}", self.radius);
        let material_key = format!("color_{:02x}{:02x}{:02x}_{:03x}:{:03x}:{:03x}", self.color.r, self.color.g, self.color.b, self.light.r, self.light.g, self.light.b);

        let mesh_handle = cache.meshes.entry(mesh_key.clone()).or_insert_with(|| {
            meshes.add(Sphere::new(1.0f32).mesh().ico(5).unwrap())
        }).clone();

        let material_handle = cache.materials.entry(material_key.clone()).or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: color,
                emissive: LinearRgba::rgb(
                    self.light.r as f32 / 255.0,
                    self.light.g as f32 / 255.0,
                    self.light.b as f32 / 255.0,
                ),
                ..Default::default()
            })
        }).clone();

        // TODO: Adjust the light for scale changes
        let light_color = Color::srgb(self.light.r as f32 / 255.0, self.light.g as f32 / 255.0, self.light.b as f32 / 255.0);
        let light = PointLight {
            color: light_color,
            intensity: self.intensity,
            range: 10000.0,
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