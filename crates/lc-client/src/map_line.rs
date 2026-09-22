//! The map's lines: tube meshes whose width the vertex shader holds in pixels.
//!
//! The width is worked out per vertex from the view, so the materials hold only constants and
//! are never written per frame; writing a few hundred of them every frame was half a 21 ms
//! frame. A vertex is sized by the camera's distance to the nearer of the two center-line
//! segments meeting there, not to the vertex itself: a spoke is one long segment, and sized by
//! its ends the part passing under the camera draws as a band. A tube is then never wider than
//! `d · rad_per_px · width_px`, so it cannot contain the camera.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use em_render::body_material::BASE_TUBE_RADIUS;
use em_render::wire_mesh::{ATTRIBUTE_ARC_LENGTH, ATTRIBUTE_CENTER_AFTER, ATTRIBUTE_CENTER_BEFORE};

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, PartialEq)]
pub struct MapLineMaterial {
    #[uniform(0)]
    pub base_color: LinearRgba,
    /// Scaled by the mesh's vertex alpha, which is how grid lines and equators differ.
    #[uniform(0)]
    pub emission_strength: f32,
    #[uniform(0)]
    pub base_tube_radius: f32,
    /// The most of its own unit mesh a tube may take. See [`tube_radius`].
    #[uniform(0)]
    pub max_fraction: f32,
    #[uniform(0)]
    pub width_px: f32,
    /// Dash length on screen, with gaps as long; zero for a solid line.
    #[uniform(0)]
    pub dash_px: f32,
}

impl Default for MapLineMaterial {
    fn default() -> Self {
        Self {
            base_color: LinearRgba::new(0.5, 0.5, 0.5, 1.0),
            emission_strength: 1.0,
            base_tube_radius: BASE_TUBE_RADIUS,
            max_fraction: 1.0,
            width_px: 1.0,
            dash_px: 0.0,
        }
    }
}

impl Material for MapLineMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/map_line.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/map_line.wgsl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
            ATTRIBUTE_CENTER_BEFORE.at_shader_location(6),
            ATTRIBUTE_CENTER_AFTER.at_shader_location(7),
            ATTRIBUTE_ARC_LENGTH.at_shader_location(8),
        ])?];
        Ok(())
    }
}

pub struct MapLinePlugin;

impl Plugin for MapLinePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<MapLineMaterial>::default());
    }
}

/// The tube radius at one vertex, in mesh units. `map_line.wgsl` repeats this, [`nearest`] and
/// [`dash_period`]; change them together.
///
/// `scale` is render units per mesh unit across the tube. The cap keeps a mark smaller than a
/// line's width from turning into a blob of tube.
pub fn tube_radius(scale: f32, rad_per_px: f32, distance: f32, max_fraction: f32, width_px: f32)
    -> f32 {
    let world = (distance * rad_per_px * width_px).max(f32::MIN_POSITIVE);
    match scale > f32::MIN_POSITIVE {
        true => (world / scale).min(max_fraction),
        false => BASE_TUBE_RADIUS,
    }
}

/// Distance from `from` to the segment `a`-`b`.
pub fn nearest(from: Vec3, a: Vec3, b: Vec3) -> f32 {
    let span = b - a;
    let along = match span.length_squared() > f32::MIN_POSITIVE {
        true => ((from - a).dot(span) / span.length_squared()).clamp(0.0, 1.0),
        false => 0.0,
    };
    from.distance(a + span * along)
}

/// The two dash periods the shader fades between, in arc length, and the fade. `per_px` is arc
/// length per screen pixel.
pub fn dash_period(dash_px: f32, per_px: f32) -> (f32, f32, f32) {
    let level = (2.0 * dash_px * per_px).log2();
    let period = level.floor().exp2();
    (period, 2.0 * period, level.fract())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A long segment passing just under the camera: sized by its ends it would be a band.
    #[test]
    fn a_long_segment_is_sized_by_the_part_nearest_the_camera() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        let (a, b) = (Vec3::new(-20.0, 0.0, 0.0), Vec3::new(20.0, 0.0, 0.0));
        let eye = Vec3::new(0.3, 0.05, 0.0);
        let (fraction, width) = (1.0, 1.6);

        let nearest_px = eye.distance(Vec3::new(0.3, 0.0, 0.0)) * rad_per_px * width;
        for end in [a, b] {
            let reach = nearest(eye, a, b);
            let radius = tube_radius(1.0, rad_per_px, reach, fraction, width);
            assert!(radius <= nearest_px * 1.0001, "an end drew {radius:e} for {nearest_px:e}");
            let by_the_end = tube_radius(1.0, rad_per_px, eye.distance(end), fraction, width);
            assert!(by_the_end > radius * 100.0, "premise: the ends alone would be a band");
        }
    }

    /// At any scale the shorter period lights half to all of the dash asked for, the longer one
    /// to two times it.
    #[test]
    fn a_dash_stays_near_its_length_on_screen() {
        let dash_px = 5.0;
        for exponent in -40..40 {
            let per_px = 1.37f32.powi(exponent);
            let (short, long, _) = dash_period(dash_px, per_px);
            let (short_px, long_px) = (short * 0.5 / per_px, long * 0.5 / per_px);
            assert!(short_px > dash_px * 0.5 - 1.0e-3 && short_px <= dash_px * 1.0001,
                "{short_px} px at {per_px:e}");
            assert!(long_px > dash_px * 0.9999 && long_px <= dash_px * 2.0001,
                "{long_px} px at {per_px:e}");
        }
    }

    #[test]
    fn the_nearest_point_of_a_segment_is_on_it() {
        let (a, b) = (Vec3::ZERO, Vec3::X);
        assert!((nearest(Vec3::new(0.5, 1.0, 0.0), a, b) - 1.0).abs() < 1.0e-6);
        assert!((nearest(Vec3::new(-3.0, 4.0, 0.0), a, b) - 5.0).abs() < 1.0e-6, "past an end");
        assert!((nearest(Vec3::new(0.0, 2.0, 0.0), a, a) - 2.0).abs() < 1.0e-6, "a point");
    }
}
