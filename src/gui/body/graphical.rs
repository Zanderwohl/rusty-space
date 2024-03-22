use bevy::pbr::{PbrBundle, StandardMaterial};
use bevy::prelude::{Assets, Color, Component, default, Mesh, ResMut, Sphere, Transform};
use glam::DVec3;
use crate::body::body::Body;
use crate::body::fixed::FixedBody;

pub trait Renderable {
    fn world_space(&self, scale: f64) -> DVec3;

    fn mesh(&self,
            meshes: &mut ResMut<Assets<Mesh>>,
            materials: &mut ResMut<Assets<StandardMaterial>>
    ) -> PbrBundle;
}

impl Renderable for dyn Body {
    fn world_space(&self, scale: f64) -> DVec3 {
        let real_position = self.global_position();
        real_position * scale
    }

    fn mesh(&self,
            mut meshes: &mut ResMut<Assets<Mesh>>,
            mut materials: &mut ResMut<Assets<StandardMaterial>>
    ) -> PbrBundle {
        PbrBundle {
            mesh: meshes.add(Sphere::new(1.0)),
            material: materials.add(Color::rgb(5.0, 2.5, 0.3)),
            transform: Transform::from_xyz(0.0, 0.5, 0.0),
            ..default()
        }
    }
}
