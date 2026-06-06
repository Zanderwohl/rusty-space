use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;
use bevy::asset::RenderAssetUsages;
use bevy::color::LinearRgba;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use serde::{Deserialize, Serialize};
use crate::presentation::BodyWireframeMaterial;

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
    #[serde(default)]
    pub highlight_latitudes: Vec<f64>,
}

impl DebugBall {
    /// Get highlight latitudes (for wireframe rendering).
    /// Returns the stored values, or empty if none configured.
    pub fn highlight_latitudes(&self) -> Vec<f64> {
        self.highlight_latitudes.clone()
    }
}

impl DebugBall {
    /// Creates a black unlit occluder sphere at radius 0.97.
    /// The wireframe grid is spawned separately as a child entity.
    pub fn pbr_bundle(&self,
                      cache: &mut ResMut<AssetCache>,
                      meshes: &mut Assets<Mesh>,
                      materials: &mut Assets<StandardMaterial>,
                      _images: &mut ResMut<Assets<Image>>,
    ) -> (Mesh3d, MeshMaterial3d<StandardMaterial>) {
        let mesh_key = "occluder_sphere".to_string();
        let material_key = "occluder_black".to_string();

        let mesh_handle = cache.meshes.entry(mesh_key).or_insert_with(|| {
            meshes.add(Sphere::new(0.97f32).mesh().ico(5).unwrap())
        }).clone();

        let material_handle = cache.materials.entry(material_key).or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: Color::BLACK,
                unlit: true,
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

/// Build an empty mesh that still declares the vertex layout required by
/// `body_wireframe.wgsl` (`position`, `normal`, `color`).
fn empty_wireframe_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        VertexAttributeValues::Float32x4(Vec::new()),
    );
    mesh.insert_indices(Indices::U32(Vec::new()));
    mesh
}

/// Find two perpendicular vectors to form a plane orthogonal to the given direction.
fn perpendicular_vectors(dir: Vec3) -> (Vec3, Vec3) {
    let not_parallel = if dir.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let perp1 = dir.cross(not_parallel).normalize_or_zero();
    let perp2 = dir.cross(perp1).normalize_or_zero();
    (perp1, perp2)
}

/// Build tube geometry for a single line segment (two rings).
fn build_tube_segment(
    start: Vec3,
    end: Vec3,
    brightness: f32,
    tube_radius: f32,
    tube_sides: u32,
    index_offset: u32,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<u32>) {
    if tube_sides < 3 {
        return (vec![], vec![], vec![], vec![]);
    }

    let tangent = (end - start).normalize_or_zero();
    if tangent.length_squared() < 0.001 {
        return (vec![], vec![], vec![], vec![]);
    }

    let (perp1, perp2) = perpendicular_vectors(tangent);

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity((tube_sides * 2) as usize);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity((tube_sides * 2) as usize);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity((tube_sides * 2) as usize);
    let mut indices: Vec<u32> = Vec::with_capacity((tube_sides * 6) as usize);

    for center in [start, end] {
        for i in 0..tube_sides {
            let angle = (i as f32 / tube_sides as f32) * 2.0 * PI;
            let (sin_a, cos_a) = angle.sin_cos();
            let offset = perp1 * cos_a * tube_radius + perp2 * sin_a * tube_radius;
            let pos = center + offset;
            let normal = offset.normalize_or_zero();

            positions.push([pos.x, pos.y, pos.z]);
            normals.push([normal.x, normal.y, normal.z]);
            colors.push([1.0, 1.0, 1.0, brightness]);
        }
    }

    let start_base = index_offset;
    let end_base = index_offset + tube_sides;
    for i in 0..tube_sides {
        let i_next = (i + 1) % tube_sides;

        indices.push(start_base + i);
        indices.push(end_base + i);
        indices.push(start_base + i_next);

        indices.push(start_base + i_next);
        indices.push(end_base + i);
        indices.push(end_base + i_next);
    }

    (positions, normals, colors, indices)
}

/// Generate a wireframe icosphere by creating tube segments on all triangle edges.
fn generate_icosphere_wireframe_mesh(subdivisions: u32, tube_radius: f32, tube_sides: u32) -> Mesh {
    let source_mesh = match Sphere::new(1.0f32).mesh().ico(subdivisions) {
        Ok(mesh) => mesh,
        Err(_) => return empty_wireframe_mesh(),
    };

    let positions = match source_mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(values)) => values
            .iter()
            .map(|p| Vec3::new(p[0], p[1], p[2]).normalize_or_zero())
            .collect::<Vec<_>>(),
        _ => return empty_wireframe_mesh(),
    };
    if positions.is_empty() {
        return empty_wireframe_mesh();
    }

    let tri_indices: Vec<u32> = match source_mesh.indices() {
        Some(Indices::U16(indices)) => indices.iter().map(|&i| i as u32).collect(),
        Some(Indices::U32(indices)) => indices.clone(),
        None => return empty_wireframe_mesh(),
    };
    if tri_indices.len() < 3 {
        return empty_wireframe_mesh();
    }

    let mut unique_edges: HashSet<(u32, u32)> = HashSet::new();
    for tri in tri_indices.chunks_exact(3) {
        let edges = [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])];
        for (a, b) in edges {
            let edge = if a < b { (a, b) } else { (b, a) };
            unique_edges.insert(edge);
        }
    }

    let mut all_positions: Vec<[f32; 3]> = Vec::new();
    let mut all_normals: Vec<[f32; 3]> = Vec::new();
    let mut all_colors: Vec<[f32; 4]> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();

    for (a, b) in unique_edges {
        let Some(&start) = positions.get(a as usize) else {
            continue;
        };
        let Some(&end) = positions.get(b as usize) else {
            continue;
        };

        let offset = all_positions.len() as u32;
        let (pos, norm, col, idx) =
            build_tube_segment(start, end, 1.0, tube_radius, tube_sides, offset);
        all_positions.extend(pos);
        all_normals.extend(norm);
        all_colors.extend(col);
        all_indices.extend(idx);
    }

    if all_positions.is_empty() {
        return empty_wireframe_mesh();
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, all_positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, all_normals);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        VertexAttributeValues::Float32x4(all_colors),
    );
    mesh.insert_indices(Indices::U32(all_indices));
    mesh
}

#[derive(Serialize, Deserialize, Clone)]
pub struct StarBall {
    pub radius: f64,
    pub color: AppearanceColor,
    pub light: AppearanceColor,
    pub absolute_magnitude: f32,
}

impl StarBall {
    const STAR_WIREFRAME_SUBDIVISIONS: u32 = 2;
    const STAR_WIREFRAME_TUBE_RADIUS: f32 = 0.015;
    const STAR_WIREFRAME_TUBE_SIDES: u32 = 4;
    const STAR_GLOW_EMISSION_STRENGTH: f32 = 10.0;

    pub fn intensity(&self) -> f32 {
        // Convert absolute magnitude to luminous flux (lumens) relative to the Sun
        const SUN_ABSOLUTE_MAGNITUDE: f64 = 4.83;
        const SUN_LUMINOUS_FLUX_LM: f64 = 3.5e28;
        let m = self.absolute_magnitude as f64;
        let luminosity_ratio = 10f64.powf(0.4 * (SUN_ABSOLUTE_MAGNITUDE - m));
        (SUN_LUMINOUS_FLUX_LM * luminosity_ratio) as f32
    }

    pub fn emissive_luminance(&self) -> f32 {
        // Approximate solar surface luminance in nits (cd/m^2), scaled by absolute magnitude
        // L_sun ≈ 1.8e9 nits at the photosphere
        const SUN_ABSOLUTE_MAGNITUDE: f64 = 4.83;
        const SUN_SURFACE_LUMINANCE_NITS: f64 = 1.83e9;
        let m = self.absolute_magnitude as f64;
        let luminosity_ratio = 10f64.powf(0.4 * (SUN_ABSOLUTE_MAGNITUDE - m));
        (SUN_SURFACE_LUMINANCE_NITS * luminosity_ratio) as f32
    }
    
    pub fn pbr_bundle(&self,
                      cache: &mut ResMut<AssetCache>,
                      meshes: &mut Assets<Mesh>,
                      materials: &mut Assets<BodyWireframeMaterial>,
                      _images: &mut ResMut<Assets<Image>>,
    ) -> (Mesh3d, MeshMaterial3d<BodyWireframeMaterial>, PointLight) {
        let mesh_key = format!(
            "star_wire_ico_sub{}_tube{}_sides{}",
            Self::STAR_WIREFRAME_SUBDIVISIONS,
            Self::STAR_WIREFRAME_TUBE_RADIUS,
            Self::STAR_WIREFRAME_TUBE_SIDES
        );

        let mesh_handle = cache.meshes.entry(mesh_key.clone()).or_insert_with(|| {
            let mesh = generate_icosphere_wireframe_mesh(
                Self::STAR_WIREFRAME_SUBDIVISIONS,
                Self::STAR_WIREFRAME_TUBE_RADIUS,
                Self::STAR_WIREFRAME_TUBE_SIDES,
            );
            meshes.add(mesh)
        }).clone();

        let material_handle = materials.add(BodyWireframeMaterial {
            base_color: LinearRgba::new(
                self.color.r as f32 / 255.0,
                self.color.g as f32 / 255.0,
                self.color.b as f32 / 255.0,
                1.0,
            ),
            emission_strength: Self::STAR_GLOW_EMISSION_STRENGTH,
            alpha_mode: AlphaMode::Opaque,
            ..Default::default()
        });

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