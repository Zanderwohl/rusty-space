use bevy::pbr::{PbrBundle, StandardMaterial};
use bevy::prelude::{Assets, Color, default, Mesh, ResMut, Sphere, Transform};
use glam::{DVec3, Vec3};
use crate::body::body::Body;
use crate::body::fixed::FixedBody;

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
            material: materials.add(Color::rgb(5.0 * 2.0, 2.5 * 2.0, 0.3 * 2.0)),
            transform: Transform::from_xyz(0.0, 0.5, 0.0),
            ..default()
        }
    }
}
