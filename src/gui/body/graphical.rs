use bevy::pbr::{PbrBundle, PointLight, PointLightBundle, StandardMaterial};
use bevy::prelude::{Assets, Color, Commands, Component, default, Mesh, ResMut, Sphere, Transform};
use glam::Vec3;
use bevy::hierarchy::BuildChildren;
use crate::body::body::Body;
use crate::body::fixed::FixedBody;
use crate::gui::body::engine::VisibleBody;

pub trait Renderable {
    /// We must scale to a screen scale,
    /// then reduce precision to Vec3.
    /// This is too low for calcs but should be not too jittery for display.
    fn world_space(&self, scale: f64) -> Vec3;

    fn mesh(&self,
            meshes: &mut Assets<Mesh>,
            materials: &mut Assets<StandardMaterial>
    ) -> PbrBundle;
}

impl Renderable for FixedBody {
    fn world_space(&self, scale: f64) -> Vec3 {
        let real_position = self.global_position();
        (real_position * scale).as_vec3()
    }

    fn mesh(&self,
            meshes: &mut Assets<Mesh>,
            materials: &mut Assets<StandardMaterial>
    ) -> PbrBundle {
        println!("Creating basic mesh.");
        PbrBundle {
            mesh: meshes.add(Sphere::new(1.0)),
            material: materials.add(Color::rgb(5.0 * 3.0, 2.5 * 3.0, 0.3 * 3.0)),
            transform: Transform::from_xyz(0.0, 0.5, 0.0),
            ..default()
        }
    }
}

pub fn create_star_mesh<ScreenTrait: Component + Default>(commands: &mut Commands, meshes: &mut ResMut<Assets<Mesh>>, materials: &mut ResMut<Assets<StandardMaterial>>, star: FixedBody) {
    commands.spawn((star.mesh(&mut *meshes, &mut *materials), VisibleBody, ScreenTrait::default(), star))
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
