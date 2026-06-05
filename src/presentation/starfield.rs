//! Starfield background rendering system.

use std::collections::HashSet;
use bevy::prelude::*;
use bevy::render::storage::ShaderStorageBuffer;

use bevy::color::Color;
use crate::catalog::Catalogs;
use crate::catalog::spectral::SpectralType;
use crate::gui::settings::Settings;
use crate::presentation::starfield_material::StarfieldMaterial;

/// Marker component for the starfield sphere entity.
#[derive(Component)]
pub struct Starfield;

/// Resource holding the settings buffer handle for runtime updates.
#[derive(Resource)]
pub struct StarfieldSettingsBuffer(pub Handle<ShaderStorageBuffer>);

/// Spawns the starfield background sphere with all catalog stars.
pub fn spawn_starfield(
    mut commands: Commands,
    catalogs: Res<Catalogs>,
    settings: Res<Settings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut materials: ResMut<Assets<StarfieldMaterial>>,
) {
    // Combine closest and brightest stars, deduplicating by id
    let mut seen_ids = HashSet::new();
    let mut star_data: Vec<[f32; 4]> = Vec::new();
    let mut color_data: Vec<[f32; 4]> = Vec::new();

    let default_color = Color::linear_rgba(1.0, 1.0, 1.0, 1.0);

    for star in catalogs.hyg_closest.iter().chain(catalogs.hyg_brightest.iter()) {
        if seen_ids.insert(star.id) {
            star_data.push([
                star.gpu.dir[0],
                star.gpu.dir[1],
                star.gpu.dir[2],
                star.gpu.mag,
            ]);

            let color = star.spectral.as_ref()
                .map(SpectralType::to_color)
                .unwrap_or(default_color);
            let linear = color.to_linear();
            color_data.push([linear.red, linear.green, linear.blue, 1.0]);
        }
    }

    let star_count = star_data.len() as u32;
    info!("Uploading {} unique stars to GPU buffer", star_count);

    // Upload star data and colors to GPU storage buffers
    let stars_buffer = buffers.add(ShaderStorageBuffer::from(star_data));
    let colors_buffer = buffers.add(ShaderStorageBuffer::from(color_data));

    // Create settings buffer: [brightness, padding, padding, padding]
    let settings_data: Vec<[f32; 4]> = vec![[settings.display.star_brightness, 0.0, 0.0, 0.0]];
    let settings_buffer = buffers.add(ShaderStorageBuffer::from(settings_data));

    // Store the settings buffer handle for runtime updates
    commands.insert_resource(StarfieldSettingsBuffer(settings_buffer.clone()));

    // Create the starfield material
    let material_handle = materials.add(StarfieldMaterial {
        stars: stars_buffer,
        colors: colors_buffer,
        settings: settings_buffer,
        star_count,
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

/// Syncs the star brightness setting to the starfield settings buffer.
pub fn update_starfield_brightness(
    settings: Res<Settings>,
    handle: Option<Res<StarfieldSettingsBuffer>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    let Some(handle) = handle else { return };
    if let Some(buffer) = buffers.get_mut(&handle.0) {
        let data: Vec<[f32; 4]> = vec![[settings.display.star_brightness, 0.0, 0.0, 0.0]];
        *buffer = ShaderStorageBuffer::from(data);
    }
}
