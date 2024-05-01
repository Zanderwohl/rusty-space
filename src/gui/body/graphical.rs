use bevy::pbr::{PbrBundle, PointLight, PointLightBundle, StandardMaterial};
use bevy::prelude::{Assets, Color, Commands, Component, Cylinder, default, Entity, Mesh, ResMut, Sphere, Transform};
use glam::{DVec3, Vec3};
use bevy::hierarchy::BuildChildren;
use bevy::log::info;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use num_traits::FloatConst;
use crate::body::universe::Body;
use crate::gui::editor::editor::BodyId;

#[bevy_trait_query::queryable]
pub trait Renderable {
    /// We must scale to a screen scale,
    /// then reduce precision to Vec3.
    /// This is too low for calcs but should be not too jittery for display.
    fn world_space(&self, global_position: DVec3, scale: f64) -> Vec3 {
        (global_position * scale).as_vec3()
    }
}


pub fn spawn_as_star<ScreenTrait: Component + Default>(body_id: u32, body: &Body, commands: &mut Commands, meshes: &mut ResMut<Assets<Mesh>>, materials: &mut ResMut<Assets<StandardMaterial>>) -> Entity {
    let star_mesh = PbrBundle {
        mesh: meshes.add(Sphere::new(body.radius as f32)),
        material: materials.add(Color::rgb(5.0 * 3.0, 2.5 * 3.0, 0.3 * 3.0)),
        transform: Transform::IDENTITY,
        ..default()
    };
    commands.spawn((star_mesh, ScreenTrait::default(), BodyId(body_id)))
        .with_children(|children| {
            children.spawn(PointLightBundle {
                point_light: PointLight {
                    radius: 100.0,
                    color: Color::rgb(1.0, 0.3, 0.1),
                    ..default()
                },
                ..default()
            });
        }).id()
}

pub fn spawn_as_planet<ScreenTrait: Component + Default>(
    body_id: u32,
    body: &Body,
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>) -> Entity {
    let planet_mesh = PbrBundle {
        mesh: meshes.add(Sphere::new(body.radius as f32)),
        material: materials.add(Color::rgb(0.2, 0.4, 0.8)),
        transform: Transform::IDENTITY,
        ..default()
    };
    commands.spawn((planet_mesh, ScreenTrait::default(), BodyId(body_id))).id()
}

pub fn spawn_as_ring_hab<ScreenTrait: Component + Default>(
    body_id: u32,
    body: &Body,
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>) -> Entity {
    let ring_mesh = PbrBundle {
        mesh: meshes.add(ring_mesh(3.0, 0.1, 0.02)),
        material: materials.add(Color::rgb(0.2, 0.4, 0.8)),
        transform: Transform::IDENTITY,
        ..default()
    };
    commands.spawn((ring_mesh, ScreenTrait::default(), BodyId(body_id))).id()
}

fn dynamic_cube_mesh(radius: f32, height: f32, thickness: f32) -> Mesh {
    let mut triangles = vec![
        // top (facing towards +y)
        [-0.5, 0.5, -0.5], // vertex with index 0
        [0.5, 0.5, -0.5], // vertex with index 1
        [0.5, 0.5, 0.5], // etc. until 23
        [-0.5, 0.5, 0.5],
        // bottom   (-y)
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, -0.5, 0.5],
        [-0.5, -0.5, 0.5],
        // right    (+x)
        [0.5, -0.5, -0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5], // This vertex is at the same position as vertex with index 2, but they'll have different UV and normal
        [0.5, 0.5, -0.5],
        // left     (-x)
        [-0.5, -0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [-0.5, 0.5, 0.5],
        [-0.5, 0.5, -0.5],
        // back     (+z)
        [-0.5, -0.5, 0.5],
        [-0.5, 0.5, 0.5],
        [0.5, 0.5, 0.5],
        [0.5, -0.5, 0.5],
        // forward  (-z)
        [-0.5, -0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [0.5, 0.5, -0.5],
        [0.5, -0.5, -0.5],
    ];
    let mut uv = vec![
        // Assigning the UV coords for the top side.
        [0.0, 0.2], [0.0, 0.0], [1.0, 0.0], [1.0, 0.25],
        // Assigning the UV coords for the bottom side.
        [0.0, 0.45], [0.0, 0.25], [1.0, 0.25], [1.0, 0.45],
        // Assigning the UV coords for the right side.
        [1.0, 0.45], [0.0, 0.45], [0.0, 0.2], [1.0, 0.2],
        // Assigning the UV coords for the left side.
        [1.0, 0.45], [0.0, 0.45], [0.0, 0.2], [1.0, 0.2],
        // Assigning the UV coords for the back side.
        [0.0, 0.45], [0.0, 0.2], [1.0, 0.2], [1.0, 0.45],
        // Assigning the UV coords for the forward side.
        [0.0, 0.45], [0.0, 0.2], [1.0, 0.2], [1.0, 0.45],
    ];
    let mut normals = vec![
        // Normals for the top side (towards +y)
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        // Normals for the bottom side (towards -y)
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        // Normals for the right side (towards +x)
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        // Normals for the left side (towards -x)
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        // Normals for the back side (towards +z)
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        // Normals for the forward side (towards -z)
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
    ];
    let mut indicies= vec![0,3,1 , 1,3,2, // triangles making up the top (+y) facing side.
                                   4,5,7 , 5,6,7, // bottom (-y)
                                   8,11,9 , 9,11,10, // right (+x)
                                   12,13,15 , 13,14,15, // left (-x)
                                   16,19,17 , 17,19,18, // back (+z)
                                   20,21,23 , 21,22,23, // forward (-z)
                                   ];

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, triangles)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_indices(Indices::U32(indicies))
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

