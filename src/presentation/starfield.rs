//! Starfield background rendering system.
//!
//! Each catalog star is rendered as a single camera-facing billboard quad.
//! All per-star data (direction, color, baked brightness, size factor) is baked
//! into the mesh vertex attributes once at spawn; the shader expands the quad
//! and applies a radial falloff. This replaces the previous full-sphere shader
//! that looped over every star for every fragment.

use std::collections::HashSet;
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};

use bevy::color::Color;
use crate::catalog::Catalogs;
use crate::catalog::spectral::SpectralType;
use crate::gui::settings::Settings;
use crate::presentation::starfield_material::{
    StarfieldMaterial, StarfieldMaterialUniform, ATTRIBUTE_STAR_COLOR, ATTRIBUTE_STAR_CORNER,
    ATTRIBUTE_STAR_SIZE_T,
};

/// Marker component for the starfield quad entity.
#[derive(Component)]
pub struct Starfield;

// Magnitude range mapped onto the [star_radius_min, star_radius_max] size range.
// Brighter (lower magnitude) stars are drawn larger.
const MAG_BRIGHT: f32 = -1.5; // ~Sirius -> max radius
const MAG_FAINT: f32 = 6.0; // naked-eye limit -> min radius

// Brightness scaling: maps magnitude to HDR output.
// Sirius (mag -1.5) -> ~2.0, naked eye limit (mag 6) -> ~0.05
const BRIGHTNESS_SCALE: f32 = 0.15;
const MAG_ZERO_BRIGHTNESS: f32 = 1.0;

/// Quad corner offsets (also the radial falloff coordinate).
const QUAD_CORNERS: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

/// Spawns the starfield as a single mesh of billboard quads, one per catalog star.
pub fn spawn_starfield(
    mut commands: Commands,
    catalogs: Res<Catalogs>,
    settings: Res<Settings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StarfieldMaterial>>,
) {
    // Combine closest and brightest stars, deduplicating by id.
    let mut seen_ids = HashSet::new();

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut corners: Vec<[f32; 2]> = Vec::new();
    let mut size_ts: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let default_color = Color::linear_rgba(1.0, 1.0, 1.0, 1.0);

    for star in catalogs.hyg_closest.iter().chain(catalogs.hyg_brightest.iter()) {
        if !seen_ids.insert(star.id) {
            continue;
        }

        let dir = star.gpu.dir;
        let mag = star.gpu.mag;

        // Brightness folded into vertex color alpha (lower mag = brighter).
        let brightness = MAG_ZERO_BRIGHTNESS * 2.512f32.powf(-mag) * BRIGHTNESS_SCALE;

        // Size factor from magnitude; expanded against the radius uniforms in-shader.
        let size_t = ((MAG_FAINT - mag) / (MAG_FAINT - MAG_BRIGHT)).clamp(0.0, 1.0);

        let color = star
            .spectral
            .as_ref()
            .map(SpectralType::to_color)
            .unwrap_or(default_color);
        let linear = color.to_linear();

        let base = positions.len() as u32;
        for corner in &QUAD_CORNERS {
            positions.push(dir);
            colors.push([linear.red, linear.green, linear.blue, brightness]);
            corners.push(*corner);
            size_ts.push(size_t);
        }

        // Two triangles: (0,1,2) and (0,2,3).
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    let star_count = (positions.len() / 4) as u32;
    info!("Building starfield mesh with {} billboard stars", star_count);

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(ATTRIBUTE_STAR_COLOR, VertexAttributeValues::Float32x4(colors));
    mesh.insert_attribute(ATTRIBUTE_STAR_CORNER, VertexAttributeValues::Float32x2(corners));
    mesh.insert_attribute(ATTRIBUTE_STAR_SIZE_T, VertexAttributeValues::Float32(size_ts));
    mesh.insert_indices(Indices::U32(indices));

    let mesh_handle = meshes.add(mesh);

    let material_handle = materials.add(StarfieldMaterial {
        uniforms: StarfieldMaterialUniform {
            star_count,
            brightness: settings.display.star_brightness,
            star_radius_min: settings.display.star_radius_min,
            star_radius_max: settings.display.star_radius_max.max(settings.display.star_radius_min),
        },
    });

    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material_handle),
        Transform::default(),
        NoFrustumCulling,
        Starfield,
    ));
}

/// Syncs the Distant Objects settings (star brightness and star radius range)
/// to the starfield material uniform. Only rewrites the uniform when a value
/// actually changed, to avoid re-uploading the buffer every frame.
pub fn update_starfield_brightness(
    settings: Res<Settings>,
    starfields: Query<&MeshMaterial3d<StarfieldMaterial>, With<Starfield>>,
    mut materials: ResMut<Assets<StarfieldMaterial>>,
) {
    let brightness = settings.display.star_brightness;
    let radius_min = settings.display.star_radius_min;
    let radius_max = settings.display.star_radius_max.max(radius_min);

    for material_handle in &starfields {
        let needs_update = materials
            .get(&material_handle.0)
            .map(|m| {
                m.uniforms.brightness != brightness
                    || m.uniforms.star_radius_min != radius_min
                    || m.uniforms.star_radius_max != radius_max
            })
            .unwrap_or(false);
        if needs_update {
            if let Some(material) = materials.get_mut(&material_handle.0) {
                material.uniforms.brightness = brightness;
                material.uniforms.star_radius_min = radius_min;
                material.uniforms.star_radius_max = radius_max;
            }
        }
    }
}
