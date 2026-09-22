//! The map's lines: a tube mesh whose width the vertex shader holds in pixels.
//!
//! The width used to be a uniform the map wrote per entity per frame, from the entity's
//! distance and scale. Every write re-prepares a material and re-specializes its entity, and a
//! few hundred of them every frame was half of a 21 ms frame. The shader has the view in front
//! of it, so it sizes each vertex from that vertex's own distance and the material holds only
//! constants.
//!
//! Per segment rather than per entity, which is what the per-entity versions were
//! approximating: a ring or a shell spans a range of distances, and sizing it at one of them
//! was either too thick at the near edge — the camera ended up inside the tube — or too thin at
//! the far one, where the Oort cloud's outline broke up into dots.
//!
//! Each vertex is sized by the camera's distance to the nearer of the two center-line segments
//! meeting there, not to the vertex: a spoke is one segment forty stand-offs long, and sized by
//! its end points the part passing under the camera came out a band. Interpolated between two
//! such vertices a tube is nowhere wider than its segment's nearest point asks for, and at
//! `d · rad_per_px · width_px` that is about a three-hundredth of the distance, so a tube
//! cannot reach the camera at all.

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
    /// `base_color * (1 + weight * emission_strength)`, where the weight is the mesh's vertex
    /// alpha: grid lines and equators differ by it.
    #[uniform(0)]
    pub emission_strength: f32,
    /// The tube radius baked into the mesh, which the shader displaces from.
    #[uniform(0)]
    pub base_tube_radius: f32,
    /// The most of its own unit mesh a tube may take. See [`tube_radius`].
    #[uniform(0)]
    pub max_fraction: f32,
    #[uniform(0)]
    pub width_px: f32,
    /// Dash length on screen, with gaps as long; zero for a solid line. Held near this all
    /// along the line, perspective or not: see `map_line.wgsl`.
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

/// The tube radius at one vertex, in the mesh's own units. **`map_line.wgsl` is this function**;
/// the two must change together, and this one is what the tests hold to. So is [`nearest`].
///
/// `distance` is from the camera to the nearer of the center-line segments either side of the
/// vertex, `scale` how many render units one mesh unit is across the tube. The cap stops a mark far smaller than a
/// pixel's worth of line from becoming a blob of its own tube.
pub fn tube_radius(scale: f32, rad_per_px: f32, distance: f32, max_fraction: f32, width_px: f32)
    -> f32 {
    let world = (distance * rad_per_px * width_px).max(f32::MIN_POSITIVE);
    match scale > f32::MIN_POSITIVE {
        true => (world / scale).min(max_fraction),
        false => BASE_TUBE_RADIUS,
    }
}

/// How far `from` is from the segment `a`-`b`: the distance a vertex between two segments is
/// sized by, taken to each and the smaller kept.
pub fn nearest(from: Vec3, a: Vec3, b: Vec3) -> f32 {
    let span = b - a;
    let along = match span.length_squared() > f32::MIN_POSITIVE {
        true => ((from - a).dot(span) / span.length_squared()).clamp(0.0, 1.0),
        false => 0.0,
    };
    from.distance(a + span * along)
}

/// The two dash periods `map_line.wgsl` fades between, in arc length, and how far across.
/// `per_px` is how much arc one pixel of screen covers there.
pub fn dash_period(dash_px: f32, per_px: f32) -> (f32, f32, f32) {
    let level = (2.0 * dash_px * per_px).log2();
    let period = level.floor().exp2();
    (period, 2.0 * period, level.fract())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spoke forty units long passing a hair under the camera: sized by its ends it was a band
    /// across the view, and sized by its nearest point it is the line it should be.
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

    /// The two periods bracket the dash asked for, at any scale — the shorter lights half to
    /// all of it, the longer one to two times it — so a big shape gets more dashes rather than
    /// stretched ones.
    #[test]
    fn a_dash_stays_near_its_length_on_screen() {
        let dash_px = 5.0;
        for exponent in -40..40 {
            let per_px = 1.37f32.powi(exponent);
            let (short, long, _) = dash_period(dash_px, per_px);
            // Half a period is lit, in pixels.
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
