use bevy::pbr::{PbrBundle, PointLight, PointLightBundle, StandardMaterial};
use bevy::prelude::{Assets, Color, Commands, Component, Cylinder, default, Entity, Mesh, ResMut, Sphere, Transform};
use glam::{DVec3, Vec3};
use bevy::hierarchy::BuildChildren;
use bevy::log::info;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use num_traits::FloatConst;
use crate::body::appearance::{Appearance, Planetoid, Ring, Sun};
use crate::body::body::Body;
use crate::gui::planetarium::planetarium::BodyId;

#[bevy_trait_query::queryable]
pub trait Renderable {
    /// We must scale to a screen scale,
    /// then reduce precision to Vec3.
    /// This is too low for calcs but should be not too jittery for display.
    fn world_space(&self, global_position: DVec3, scale: f64) -> Vec3 {
        (global_position * scale).as_vec3()
    }
}

pub fn spawn_bevy<ScreenTrait: Component + Default>(body_id: u32, body: &Body, commands: &mut Commands, meshes: &mut ResMut<Assets<Mesh>>, materials: &mut ResMut<Assets<StandardMaterial>>) -> Entity {
    match &body.appearance {
        Appearance::Planetoid(planetoid) => {
            spawn_as_planet::<ScreenTrait>(body_id, planetoid, commands, meshes, materials)
        }
        Appearance::Sun(sun) => {
            spawn_as_star::<ScreenTrait>(body_id, sun, commands, meshes, materials)
        }
        Appearance::Ring(ring_hab) => {
            spawn_as_ring_hab::<ScreenTrait>(body_id, ring_hab, commands, meshes, materials)
        }
    }
}


pub fn spawn_as_star<ScreenTrait: Component + Default>(body_id: u32, star: &Sun, commands: &mut Commands, meshes: &mut ResMut<Assets<Mesh>>, materials: &mut ResMut<Assets<StandardMaterial>>) -> Entity {
    let star_mesh = PbrBundle {
        mesh: meshes.add(Sphere::new(star.radius as f32)),
        material: materials.add(Color::rgb(star.color[0], star.color[1], star.color[2])),
        transform: Transform::IDENTITY,
        ..default()
    };
    commands.spawn((star_mesh, ScreenTrait::default(), BodyId(body_id)))
        .with_children(|children| {
            children.spawn(PointLightBundle {
                point_light: PointLight {
                    radius: 100.0,
                    color: Color::rgb(star.light[0], star.light[1], star.light[2]),
                    ..default()
                },
                ..default()
            });
        }).id()
}

pub fn spawn_as_planet<ScreenTrait: Component + Default>(
    body_id: u32,
    planet: &Planetoid,
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>) -> Entity {
    let planet_mesh = PbrBundle {
        mesh: meshes.add(Sphere::new(planet.radius as f32)),
        material: materials.add(Color::rgb(planet.color[0], planet.color[1], planet.color[2])),
        transform: Transform::IDENTITY,
        ..default()
    };
    commands.spawn((planet_mesh, ScreenTrait::default(), BodyId(body_id))).id()
}

pub fn spawn_as_ring_hab<ScreenTrait: Component + Default>(
    body_id: u32,
    ring: &Ring,
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>) -> Entity {
    let ring_mesh = PbrBundle {
        mesh: meshes.add(ring_mesh(ring.radius as f32, ring.width as f32, ring.thickness as f32)),
        material: materials.add(Color::rgb(0.2, 0.4, 0.8)),
        transform: Transform::IDENTITY,
        ..default()
    };
    commands.spawn((ring_mesh, ScreenTrait::default(), BodyId(body_id))).id()
}

fn ring_mesh(radius: f32, height: f32, thickness: f32) -> Mesh {
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut uv: Vec<[f32; 2]>  = Vec::new();
    let mut normals: Vec<[f32; 3]>  = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let half_height = height / 2.0;  // Center ring on y-axis

    let segments = 200;
    let arc_length = 1f32 / segments as f32;
    for idx in 0..segments {
        let start = idx as f32 * arc_length;
        let end = (idx as f32 + 1f32) * arc_length;
        let start_angle = start * 2.0 * f32::PI();
        let end_angle = end * 2.0 * f32::PI();

        // Underside floor
        let a = [f32::cos(start_angle) * radius, half_height, f32::sin(start_angle) * radius];
        let b = [f32::cos(start_angle) * radius, -half_height, f32::sin(start_angle) * radius];
        let c = [f32::cos(end_angle) * radius, half_height, f32::sin(end_angle) * radius];
        let d = [f32::cos(end_angle) * radius, -half_height, f32::sin(end_angle) * radius];

        // Surface floor
        let inner_radius = radius - thickness;
        let e = [f32::cos(start_angle) * inner_radius, half_height, f32::sin(start_angle) * inner_radius];
        let f = [f32::cos(start_angle) * inner_radius, -half_height, f32::sin(start_angle) * inner_radius];
        let g = [f32::cos(end_angle) * inner_radius, half_height, f32::sin(end_angle) * inner_radius];
        let h = [f32::cos(end_angle) * inner_radius, -half_height, f32::sin(end_angle) * inner_radius];

        // Calculate normals from points
        let a_b = Vec3::from(b) - Vec3::from(a);
        let a_c = Vec3::from(c) - Vec3::from(a);
        let inner_facing_normal: [f32; 3] = a_b.cross(a_c).normalize().into();
        let outer_facing_normal: [f32; 3] = (-a_b.cross(a_c).normalize()).into();
        let upward_normal: [f32; 3] = [0.0, 1.0, 0.0];
        let downward_normal: [f32; 3] = [0.0, -1.0, 0.0];

        // Underside floor
        vertices.push(a);
        vertices.push(b);
        vertices.push(c);
        vertices.push(d);

        // Surface floor
        vertices.push(e);
        vertices.push(f);
        vertices.push(g);
        vertices.push(h);

        // Top Rim
        vertices.push(a);
        vertices.push(c);
        vertices.push(e);
        vertices.push(g);

        // Bottom Rim
        vertices.push(b);
        vertices.push(d);
        vertices.push(f);
        vertices.push(h);

        normals.push(outer_facing_normal);
        normals.push(outer_facing_normal);
        normals.push(outer_facing_normal);
        normals.push(outer_facing_normal);

        normals.push(inner_facing_normal);
        normals.push(inner_facing_normal);
        normals.push(inner_facing_normal);
        normals.push(inner_facing_normal);

        normals.push(upward_normal);
        normals.push(upward_normal);
        normals.push(upward_normal);
        normals.push(upward_normal);

        normals.push(downward_normal);
        normals.push(downward_normal);
        normals.push(downward_normal);
        normals.push(downward_normal);

        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);

        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);

        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);

        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);

        let base_index = idx * 16;
        let faces = [[2, 1, 0], [3, 1, 2], [4, 5, 6], [6, 5, 7], [10, 9, 8], [9, 10, 11], [12, 13, 14], [15, 14, 13]];
        for face in faces {
            for point in face {
                indices.push(base_index + point)
            }
        }
    }

    info!("Ring completed with {} vertices and {} faces.", vertices.len(), indices.len() / 3);
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_indices(Indices::U32(indices))
}

