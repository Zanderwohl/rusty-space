use bevy::pbr::{PbrBundle, PointLight, PointLightBundle, StandardMaterial};
use bevy::prelude::{Assets, Color, Commands, Component, Cylinder, default, Entity, Mesh, ResMut, Sphere, Transform};
use glam::{DVec3, Vec3};
use bevy::hierarchy::BuildChildren;
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
        mesh: meshes.add(ring_mesh(3.0, 0.1, 0.01)),
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
    ring_mesh(0.0, 0.0, 0.0);
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, triangles)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_indices(Indices::U32(indicies))
}

fn ring_mesh(radius: f32, height: f32, thickness: f32) -> Mesh {
    let mut verticies: Vec<[f32; 3]> = Vec::new();
    let mut uv: Vec<[f32; 2]>  = Vec::new();
    let mut normals: Vec<[f32; 3]>  = Vec::new();
    let mut indicies: Vec<u32> = Vec::new();

    let half_height = height / 2.0;

    let sides = 200;
    let slice = 1f32 / sides as f32;
    for idx in 0..sides {
        let angle = idx as f32 * slice * 2.0 * f32::PI();
        let next_angle = (idx as f32 + 1f32) * slice * 2.0 * f32::PI();

        let a = [f32::cos(angle) * radius, half_height, f32::sin(angle) * radius];
        let b = [f32::cos(angle) * radius, -half_height, f32::sin(angle) * radius];
        let c = [f32::cos(next_angle) * radius, half_height, f32::sin(next_angle) * radius];
        let d = [f32::cos(next_angle) * radius, -half_height, f32::sin(next_angle) * radius];

        let inner_radius = radius - thickness;
        let e = [f32::cos(angle) * inner_radius, half_height, f32::sin(angle) * inner_radius];
        let f = [f32::cos(angle) * inner_radius, -half_height, f32::sin(angle) * inner_radius];
        let g = [f32::cos(next_angle) * inner_radius, half_height, f32::sin(next_angle) * inner_radius];
        let h = [f32::cos(next_angle) * inner_radius, -half_height, f32::sin(next_angle) * inner_radius];

        let a_b = Vec3::from(b) - Vec3::from(a);
        let a_c = Vec3::from(c) - Vec3::from(a);
        let normal: [f32; 3] = a_b.cross(a_c).normalize().into();
        let antinormal: [f32; 3] = (-a_b.cross(a_c).normalize()).into();

        verticies.push(a);
        verticies.push(b);
        verticies.push(c);
        verticies.push(d);
        verticies.push(e);
        verticies.push(f);
        verticies.push(g);
        verticies.push(h);

        normals.push(antinormal);
        normals.push(antinormal);
        normals.push(antinormal);
        normals.push(antinormal);
        normals.push(normal);
        normals.push(normal);
        normals.push(normal);
        normals.push(normal);


        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);
        uv.push([0.0, 0.0]);

        let base_index = idx * 8;
        for point in [2, 1, 0] {
            let index = base_index + point;
            indicies.push(index);
        }
        for point in [3, 1, 2] {
            let index = base_index + point;
            indicies.push(index);
        }
        for point in [4, 5, 6] {
            let index = base_index + point;
            indicies.push(index);
        }
        for point in [6, 5, 7] {
            let index = base_index + point;
            indicies.push(index);
        }
    }

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, verticies)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_indices(Indices::U32(indicies))
}

