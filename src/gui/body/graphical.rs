use bevy::pbr::{PbrBundle, PointLight, PointLightBundle, StandardMaterial};
use bevy::prelude::{Assets, Color, Commands, Component, default, Mesh, ResMut, Sphere, Transform};
use glam::{DVec3, Vec3};
use bevy::hierarchy::BuildChildren;
use bevy::time::Fixed;
use crate::body::body::Body;
use crate::body::fixed::FixedBody;
use crate::gui::body::engine::VisibleBody;
use crate::gui::editor;
use crate::gui::editor::display::Star;

pub trait Renderable {
    /// We must scale to a screen scale,
    /// then reduce precision to Vec3.
    /// This is too low for calcs but should be not too jittery for display.
    fn world_space(&self, global_position: DVec3, scale: f64) -> Vec3 {
        (global_position * scale).as_vec3()
    }
}

/// Make "renderable" for things like Star, Planet, spaceship, etc?
/// Method of propulsion has nothing to do with being fixed or moving.
impl Renderable for FixedBody {}

impl FixedBody {
    pub fn show_as_star<ScreenTrait: Component + Default>(self, commands: &mut Commands, meshes: &mut ResMut<Assets<Mesh>>, materials: &mut ResMut<Assets<StandardMaterial>>) {
        let star_mesh = PbrBundle {
            mesh: meshes.add(Sphere::new(1.0)),
            material: materials.add(Color::rgb(5.0 * 3.0, 2.5 * 3.0, 0.3 * 3.0)),
            transform: Transform::from_xyz(0.0, 0.5, 0.0),
            ..default()
        };
        commands.spawn((star_mesh, Star, ScreenTrait::default(), self))
            .with_children(|children| {
                children.spawn(PointLightBundle {
                    point_light: PointLight {
                        radius: 100.0,
                        color: Color::rgb(1.0, 0.3, 0.1),
                        ..default()
                    },
                    ..default()
                });
            });
    }
}


