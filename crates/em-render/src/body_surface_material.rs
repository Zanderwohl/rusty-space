//! A resolved body's surface, generated rather than stored.
//!
//! No textures. What a body looks like follows from its radius, mass and temperature, which
//! every body in every system already has; everything past that is a seed. The host supplies
//! `shaders/body_surface.wgsl`.

use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct BodySurfaceUniform {
    pub dark: Vec4,
    pub light: Vec4,
    /// World direction to the star; `w` is the ambient floor on the night side.
    pub to_star: Vec4,
    /// `(brightness, contrast, seed, banded)`.
    pub params: Vec4,
}

impl Default for BodySurfaceUniform {
    fn default() -> Self {
        Self {
            dark: Vec4::new(0.13, 0.12, 0.11, 1.0),
            light: Vec4::new(0.34, 0.32, 0.29, 1.0),
            to_star: Vec4::new(0.0, 0.0, 1.0, 0.015),
            params: Vec4::new(1.0, 0.9, 0.0, 0.0),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct BodySurfaceMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: BodySurfaceUniform,
}

impl Material for BodySurfaceMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/body_surface.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/body_surface.wgsl".into()
    }

    /// Opaque, and the only thing in this renderer that writes depth. That is what puts the far
    /// half of a ring behind its planet.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }
}

pub struct BodySurfaceMaterialPlugin;

impl Plugin for BodySurfaceMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<BodySurfaceMaterial>::default());
    }
}
