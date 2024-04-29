use bevy::pbr::{PbrBundle, PointLight, PointLightBundle, StandardMaterial};
use bevy::prelude::{Assets, Color, Commands, Component, default, Entity, Mesh, ResMut, Sphere, Transform};
use glam::{DVec3, Vec3};
use bevy::hierarchy::BuildChildren;
use crate::body::universe::Body;
use crate::gui::editor::editor::{BodyId, Star};

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
    commands.spawn((star_mesh, Star, ScreenTrait::default(), BodyId(body_id)))
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

pub fn spawn_as_planet<ScreenTrait: Component + Default,>(
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
    commands.spawn((planet_mesh, Star, ScreenTrait::default(), BodyId(body_id))).id()
}
