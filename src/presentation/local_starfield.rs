//! Local star skybox pass.
//!
//! Renders simulation stars (`Appearance::Star`) as emissive billboard quads in
//! a skybox-like pass, using each star's relative position, radius, and
//! intensity.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};

use crate::body::appearance::Appearance;
use crate::body::universe::save::ViewSettings;
use crate::camera::PlanetariumCamera;
use crate::gui::settings::Settings;
use crate::presentation::local_starfield_material::{
    LocalStarfieldMaterial, LocalStarfieldMaterialUniform, ATTRIBUTE_LOCAL_STAR_COLOR,
    ATTRIBUTE_LOCAL_STAR_CORNER, ATTRIBUTE_LOCAL_STAR_PARAMS,
};

/// Minimum billboard radius in screen pixels for local star quads.
const LOCAL_STAR_MIN_RADIUS_PX: f32 = 3.0;
/// One lightyear in meters.
const LIGHT_YEAR_M: f32 = 9.460_730_5e15;

const QUAD_CORNERS: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

#[derive(Component)]
pub struct LocalStarfield;

fn empty_local_starfield_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(
        ATTRIBUTE_LOCAL_STAR_COLOR,
        VertexAttributeValues::Float32x4(Vec::new()),
    );
    mesh.insert_attribute(
        ATTRIBUTE_LOCAL_STAR_CORNER,
        VertexAttributeValues::Float32x2(Vec::new()),
    );
    mesh.insert_attribute(
        ATTRIBUTE_LOCAL_STAR_PARAMS,
        VertexAttributeValues::Float32x3(Vec::new()),
    );
    mesh.insert_indices(Indices::U32(Vec::new()));
    mesh
}

pub fn spawn_local_starfield(
    mut commands: Commands,
    settings: Res<Settings>,
    view_settings: Res<ViewSettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LocalStarfieldMaterial>>,
) {
    let distance_scale = view_settings.distance_factor() as f32;
    let mesh_handle = meshes.add(empty_local_starfield_mesh());
    let material_handle = materials.add(LocalStarfieldMaterial {
        uniforms: LocalStarfieldMaterialUniform {
            brightness_min: settings.display.local_star_brightness_min,
            brightness_max: settings.display.local_star_brightness_max,
            min_angular_radius_rad: 0.0,
            max_intensity: 0.0,
            near_distance_bevy: 1.0 * distance_scale,
            far_distance_bevy: LIGHT_YEAR_M * distance_scale,
        },
    });

    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material_handle),
        Transform::default(),
        NoFrustumCulling,
        LocalStarfield,
    ));
}

pub fn update_local_starfield(
    settings: Res<Settings>,
    view_settings: Res<ViewSettings>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<PlanetariumCamera>>,
    stars: Query<(&Transform, &Appearance)>,
    local_starfield: Query<(&Mesh3d, &MeshMaterial3d<LocalStarfieldMaterial>), With<LocalStarfield>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LocalStarfieldMaterial>>,
) {
    let Ok((camera, camera_global, projection)) = cameras.single() else {
        return;
    };
    let Ok((mesh3d, material_handle)) = local_starfield.single() else {
        return;
    };
    let Some(viewport_size) = camera.logical_viewport_size() else {
        return;
    };

    let fov_y = match projection {
        Projection::Perspective(persp) => persp.fov,
        _ => 1.0,
    };
    let distance_scale = view_settings.distance_factor() as f32;
    let min_angular_radius_rad = (LOCAL_STAR_MIN_RADIUS_PX / viewport_size.y) * (fov_y * 0.5);
    let camera_pos = camera_global.translation();

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut corners: Vec<[f32; 2]> = Vec::new();
    let mut params: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut max_intensity = 0.0f32;

    for (transform, appearance) in stars.iter() {
        let Appearance::Star(star_ball) = appearance else {
            continue;
        };

        let rel = transform.translation - camera_pos;
        if rel.length_squared() <= 1e-12 {
            continue;
        }

        let intensity = star_ball.intensity();
        let radius = transform.scale.x.max(0.0);
        max_intensity = max_intensity.max(intensity);

        let linear = Color::srgb(
            star_ball.light.r as f32 / 255.0,
            star_ball.light.g as f32 / 255.0,
            star_ball.light.b as f32 / 255.0,
        )
        .to_linear();

        let base = positions.len() as u32;
        for corner in &QUAD_CORNERS {
            positions.push([rel.x, rel.y, rel.z]);
            colors.push([linear.red, linear.green, linear.blue, 1.0]);
            corners.push(*corner);
            params.push([intensity, radius, 0.0]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    if let Some(mesh_asset) = meshes.get_mut(&mesh3d.0) {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(
            ATTRIBUTE_LOCAL_STAR_COLOR,
            VertexAttributeValues::Float32x4(colors),
        );
        mesh.insert_attribute(
            ATTRIBUTE_LOCAL_STAR_CORNER,
            VertexAttributeValues::Float32x2(corners),
        );
        mesh.insert_attribute(
            ATTRIBUTE_LOCAL_STAR_PARAMS,
            VertexAttributeValues::Float32x3(params),
        );
        mesh.insert_indices(Indices::U32(indices));
        *mesh_asset = mesh;
    }

    if let Some(material) = materials.get_mut(material_handle.id()) {
        material.uniforms.brightness_min = settings.display.local_star_brightness_min;
        material.uniforms.brightness_max = settings.display.local_star_brightness_max;
        material.uniforms.min_angular_radius_rad = min_angular_radius_rad;
        material.uniforms.max_intensity = max_intensity;
        material.uniforms.near_distance_bevy = 1.0 * distance_scale;
        material.uniforms.far_distance_bevy = LIGHT_YEAR_M * distance_scale;
    }
}

pub fn clear_local_starfield(
    local_starfield: Query<&Mesh3d, With<LocalStarfield>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LocalStarfieldMaterial>>,
    material_handles: Query<&MeshMaterial3d<LocalStarfieldMaterial>, With<LocalStarfield>>,
) {
    for mesh3d in &local_starfield {
        if let Some(mesh_asset) = meshes.get_mut(&mesh3d.0) {
            *mesh_asset = empty_local_starfield_mesh();
        }
    }

    for handle in &material_handles {
        if let Some(material) = materials.get_mut(handle.id()) {
            material.uniforms.max_intensity = 0.0;
        }
    }
}
