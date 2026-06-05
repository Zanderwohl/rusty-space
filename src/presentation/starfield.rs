//! Starfield background rendering system.

use std::collections::HashSet;
use bevy::prelude::*;
use bevy::render::storage::ShaderStorageBuffer;

use crate::catalog::Catalogs;
use crate::presentation::starfield_material::StarfieldMaterial;

/// Marker component for the starfield sphere entity.
#[derive(Component)]
pub struct Starfield;

/// Spawns the starfield background sphere with all catalog stars.
pub fn spawn_starfield(
    mut commands: Commands,
    catalogs: Res<Catalogs>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut materials: ResMut<Assets<StarfieldMaterial>>,
) {
    // Combine closest and brightest stars, deduplicating by id
    let mut seen_ids = HashSet::new();
    let mut star_data: Vec<[f32; 4]> = Vec::new();

    for star in catalogs.hyg_closest.iter().chain(catalogs.hyg_brightest.iter()) {
        if seen_ids.insert(star.id) {
            // Convert StarGpuData to [f32; 4] for the GPU buffer
            star_data.push([
                star.gpu.dir[0],
                star.gpu.dir[1],
                star.gpu.dir[2],
                star.gpu.mag,
            ]);
        }
    }

    let star_count = star_data.len() as u32;
    info!("Uploading {} unique stars to GPU buffer", star_count);

    // Upload star data to GPU storage buffer
    let buffer_handle = buffers.add(ShaderStorageBuffer::from(star_data));

    // Create the starfield material
    let material_handle = materials.add(StarfieldMaterial {
        stars: buffer_handle,
        star_count,
        emission_strength: 3.0,
    });

    // Create an inverted sphere mesh (we view from inside)
    let mesh_handle = meshes.add(Sphere::new(1.0).mesh().build());

    // Spawn the starfield entity
    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material_handle),
        Transform::default(),
        Starfield,
    ));
}
