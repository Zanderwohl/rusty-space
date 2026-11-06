//! Launch tubes in a hive: hexagonal cells, each with a tube at its middle under an iris, open or
//! shut, ringed in danger tape. An open tube is drawn by tracing the view into it: its walls ringed
//! and lit by guide lights running down it, and a drone's nose at the bottom or the empty floor.
//! Laid out in the part's charts ([`crate::charts`]), so only hull-mesher meshes draw: a margin two
//! apothems wide inside each chart's edge is buffed steel, and a band takes a whole number of
//! cells around. See `assets/shaders/tubes.wgsl`.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

pub struct Tubes {
    /// A cell across its flats: two apothems.
    pub hex_m: f32,
    /// The tube's bore, and the danger tape's width around its mouth.
    pub bore_m: f32,
    pub tape_m: f32,
    /// How deep a tube runs, and how far down a loaded one holds its drone's nose.
    pub depth_m: f32,
    pub drone_m: f32,
    /// Of tubes, the share open, and of those, the share holding a drone.
    pub open: f32,
    pub loaded: f32,
    /// Guide lights' spacing down the wall, and their radiance, linear.
    pub guide_pitch_m: f32,
    pub guide: Vec3,
    /// The plate's paint, linear.
    pub paint: Vec3,
    pub roughness: f32,
    pub metallic: f32,
    pub to_star: Vec3,
    pub star: f32,
    /// The star's turn about world Y, rad/s; see [`super::spin`].
    pub star_spin: f32,
    pub to_planet: Vec3,
    /// The planet's angular radius.
    pub planet_deg: f32,
    pub planet: f32,
    pub exposure_stops: f32,
    /// Below 2^24: it reaches the shader as an f32.
    pub seed: u32,
}

impl Default for Tubes {
    fn default() -> Self {
        Self {
            hex_m: 6.0,
            bore_m: 3.6,
            tape_m: 0.45,
            depth_m: 12.0,
            drone_m: 1.2,
            open: 0.6,
            loaded: 0.6,
            guide_pitch_m: 1.2,
            guide: Vec3::new(0.35, 0.8, 1.0) * 1.5,
            paint: Vec3::new(0.3, 0.31, 0.33),
            roughness: 0.45,
            metallic: 0.3,
            to_star: super::direction(30.0, -50.0),
            star: 3.0,
            star_spin: 0.0,
            to_planet: super::direction(-15.0, 150.0),
            planet_deg: 20.0,
            planet: 0.12,
            exposure_stops: 0.0,
            seed: 5,
        }
    }
}

impl Tubes {
    /// The hull material's `(tube_cell, tube_depth, tube_guide)`; guide lights are the region's lamp.
    pub fn hull_tubes(&self) -> (Vec4, Vec4, Vec4) {
        (
            Vec4::new(self.hex_m, 0.5 * self.bore_m, self.tape_m, 0.0),
            Vec4::new(self.depth_m, self.drone_m, self.open, self.loaded),
            Vec4::new(self.guide_pitch_m, 0.0, 0.0, 0.0),
        )
    }

    pub fn material(&self) -> TubesMaterial {
        TubesMaterial {
            uniforms: TubesUniform {
                cell: Vec4::new(self.hex_m, 0.5 * self.bore_m, self.tape_m, 0.0),
                tube: Vec4::new(self.depth_m, self.drone_m, self.open, self.loaded),
                guide: self.guide.extend(self.guide_pitch_m),
                paint: self.paint.extend(0.0),
                surface: Vec4::new(self.roughness, self.metallic, 0.0, 0.0),
                to_star: self.to_star.extend(self.star_spin),
                star_color: Vec3::splat(self.star).extend(0.0),
                to_planet: self.to_planet.extend(self.planet_deg.to_radians()),
                planet_color: (Vec3::new(0.55, 0.7, 1.0) * self.planet).extend(0.0),
                ambient: Vec3::splat(0.004).extend(0.0),
                finish: Vec4::new(2f32.powf(self.exposure_stops), self.seed as f32, 0.0, 0.0),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct TubesUniform {
    cell: Vec4,
    tube: Vec4,
    guide: Vec4,
    paint: Vec4,
    surface: Vec4,
    to_star: Vec4,
    star_color: Vec4,
    to_planet: Vec4,
    planet_color: Vec4,
    ambient: Vec4,
    finish: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct TubesMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    uniforms: TubesUniform,
}

impl Material for TubesMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/tubes.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/tubes.wgsl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        super::charted(descriptor, layout)
    }
}
