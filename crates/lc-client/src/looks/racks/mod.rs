//! Server racks: on the sides, a wall of servers a deck tall between raised rack posts, a groove
//! between each server and the next; on the top and bottom, a data hall from above, rows of rack
//! tops with a fan in each, between sunken aisles. Each server wears one of [`faces`], glass that
//! mirrors the sky by day and glows green by night. Laid out in the part's own charts, which the
//! hull mesher writes ([`crate::charts`]), so only its meshes draw. See `assets/shaders/racks.wgsl`.

pub mod faces;

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

pub struct Racks {
    /// A server's width, between one post and the next.
    pub rack_m: f32,
    /// A server's height: a deck, so it reads at a building's scale.
    pub server_m: f32,
    /// A post's width, and how far its rounded faces lean at its edges, as a tangent.
    pub post_m: f32,
    pub post_lean: f32,
    /// The groove between servers: its width, and how far its walls lean, as a tangent.
    pub groove_m: f32,
    pub groove_lean: f32,
    /// How far a server's front may tip off the face's.
    pub tilt_deg: f32,
    /// On an end: a row of rack tops this deep, the aisle between rows, a rack top's fan across.
    pub depth_m: f32,
    pub aisle_m: f32,
    pub fan_m: f32,
    /// The servers' paint, linear, and how far each strays from it in brightness.
    pub paint: Vec3,
    pub spread: f32,
    pub roughness: f32,
    pub metallic: f32,
    /// A face's lit glass, linear: past 1, so it blooms.
    pub glow: Vec3,
    /// On a face the star lights, what of the glow is left.
    pub day_glow: f32,
    /// The face glass's reflectance face-on, and its roughness.
    pub glass_f0: f32,
    pub glass_roughness: f32,
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

impl Default for Racks {
    fn default() -> Self {
        Self {
            rack_m: 6.0,
            server_m: 3.0,
            post_m: 0.5,
            post_lean: 1.2,
            groove_m: 0.2,
            groove_lean: 1.0,
            tilt_deg: 1.5,
            depth_m: 4.0,
            aisle_m: 2.0,
            fan_m: 2.6,
            // The data graph's blue-gray, Oklch L 0.46 to 0.53 at chroma 0.02, in linear light.
            paint: Vec3::new(0.17, 0.19, 0.22),
            spread: 0.18,
            roughness: 0.45,
            metallic: 0.3,
            glow: Vec3::new(0.06, 1.0, 0.15) * 3.0,
            day_glow: 0.0,
            glass_f0: 0.06,
            glass_roughness: 0.1,
            to_star: super::direction(30.0, -50.0),
            star: 3.0,
            star_spin: 0.0,
            to_planet: super::direction(-15.0, 150.0),
            planet_deg: 20.0,
            planet: 0.12,
            exposure_stops: 0.0,
            seed: 11,
        }
    }
}

/// The hull material's racks: `(rack, rack_lean, rack_hall, rack_face, rack_paint, rack_glow)`.
pub struct HullRacks {
    pub rack: Vec4,
    pub lean: Vec4,
    pub hall: Vec4,
    pub face: Vec4,
    pub paint: Vec4,
    pub glow: Vec4,
}

impl Racks {
    /// Faces glow at the region's lamp in [`Self::glow`]'s hue.
    pub fn hull_racks(&self) -> HullRacks {
        let hue = self.glow / self.glow.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1.0e-6);
        HullRacks {
            rack: Vec4::new(self.rack_m, self.server_m, self.post_m, self.groove_m),
            lean: Vec4::new(self.post_lean, self.groove_lean, self.tilt_deg.to_radians(), 0.0),
            hall: Vec4::new(self.depth_m, self.aisle_m, self.fan_m, 0.0),
            face: Vec4::new(faces::FACE_M.x, faces::FACE_M.y, faces::COUNT as f32, self.day_glow),
            paint: self.paint.extend(self.spread),
            glow: hue.extend(self.glass_f0),
        }
    }

    /// Bakes the faces; see [`faces::bake`].
    pub fn material(&self, images: &mut Assets<Image>) -> RacksMaterial {
        RacksMaterial {
            faces: images.add(faces::bake()),
            uniforms: RacksUniform {
                rack: Vec4::new(self.rack_m, self.server_m, self.post_m, self.groove_m),
                lean: Vec4::new(self.post_lean, self.groove_lean, self.tilt_deg.to_radians(), 0.0),
                hall: Vec4::new(self.depth_m, self.aisle_m, self.fan_m, 0.0),
                paint: self.paint.extend(self.spread),
                surface: Vec4::new(self.roughness, self.metallic, self.glass_f0, self.glass_roughness),
                face: Vec4::new(faces::FACE_M.x, faces::FACE_M.y, faces::COUNT as f32, self.day_glow),
                glow: self.glow.extend(0.0),
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
pub struct RacksUniform {
    rack: Vec4,
    lean: Vec4,
    hall: Vec4,
    paint: Vec4,
    surface: Vec4,
    face: Vec4,
    glow: Vec4,
    to_star: Vec4,
    star_color: Vec4,
    to_planet: Vec4,
    planet_color: Vec4,
    ambient: Vec4,
    finish: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct RacksMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    uniforms: RacksUniform,
    #[texture(1, dimension = "2d_array", visibility(fragment))]
    #[sampler(2, visibility(fragment))]
    faces: Handle<Image>,
}

impl Material for RacksMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/racks.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/racks.wgsl".into()
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
