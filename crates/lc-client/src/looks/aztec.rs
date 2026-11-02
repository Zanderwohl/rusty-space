//! Azteced sheet metal: rectangular panels, each its own shade and tipped a few degrees, with
//! chamfered trenches between, and, if asked, office windows on the sides and a roof of glass hexes
//! on top: offices with a roof are the habitat. Fully procedural; see
//! `assets/shaders/aztec.wgsl`.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

const HULL_PAINT: f32 = 0.55;
const HULL_METALLIC: f32 = 0.3;
const HULL_ROUGHNESS: f32 = 0.45;

pub struct Aztec {
    /// The staggered grid before splitting, the smallest panel side, and the grid panel edges
    /// snap to, meters.
    pub cell_m: f32,
    pub min_panel_m: f32,
    pub snap_m: f32,
    /// Each wall of a seam's trench: its width across the face, and how far its normal leans
    /// toward the seam.
    pub chamfer_m: f32,
    pub lean_deg: f32,
    /// How far a panel's normal may tip off the face's.
    pub tilt_deg: f32,
    /// Of panels a little browner.
    pub brown: f32,
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
    pub windows: Option<Windows>,
    pub roof: Option<Roof>,
    /// The mesh is an axis-aligned unit cube scaled by its transform, unrotated, which is how the
    /// shader knows where a face ends, and so keeps windows [`Windows::face_margin_m`] off it.
    /// Otherwise there is no margin.
    pub unit_box: bool,
}

/// Office windows, on the sides only: decks stack along world Y. A window is placed only where it
/// fits wholly inside a panel, clear of the chamfers.
pub struct Windows {
    pub deck_m: f32,
    /// Window centers along a row.
    pub pitch_m: f32,
    /// From the deck to the window's bottom.
    pub sill_m: f32,
    pub width_m: f32,
    pub height_m: f32,
    /// No window nearer a face's edge than this.
    pub face_margin_m: f32,
    /// Of a row's runs of windows, the share lit.
    pub lit: f32,
    /// A lit window's radiance, linear.
    pub glow: Vec3,
    /// The coated glass's reflectance face-on, and its roughness.
    pub f0: f32,
    pub roughness: f32,
    /// On a face the star lights, what of a lit window's glow is left, and the star's share
    /// every pane shows, so lit and dark rooms are hard to tell apart by day.
    pub day_glow: f32,
    pub day_glass: f32,
    /// How deep the room the shader draws behind each window is; it is the window's column wide
    /// and its deck high.
    pub room_m: f32,
}

impl Default for Windows {
    fn default() -> Self {
        Self {
            deck_m: 3.0,
            pitch_m: 2.0,
            sill_m: 0.9,
            width_m: 1.6,
            height_m: 1.0,
            face_margin_m: 1.0,
            lit: 0.35,
            glow: Vec3::new(1.0, 0.82, 0.58) * 0.6,
            f0: 0.5,
            roughness: 0.12,
            day_glow: 0.05,
            day_glass: 0.02,
            room_m: 4.0,
        }
    }
}

/// The whole top as clear glass hexes in a metal frame, a dome over a forest, lit only by what of the
/// star and the planet comes in through it.
pub struct Roof {
    /// Across a hex's flats.
    pub hex_m: f32,
    /// The frame between two hexes.
    pub seam_m: f32,
    /// The chamfer where the roof meets the sides.
    pub rim_m: f32,
    /// The air's color by day, linear.
    pub sky: Vec3,
    /// The share of the star the air scatters back up through the glass.
    pub scatter: f32,
    /// The forest lies this share of the roof's width under the glass, and no deeper than
    /// `forest_m`; the stratus is part way down.
    pub depth_share: f32,
    pub forest_m: f32,
}

impl Default for Roof {
    fn default() -> Self {
        Self { hex_m: 10.0, seam_m: 0.5, rim_m: 0.5, sky: Vec3::new(0.15, 0.42, 1.0), scatter: 0.08, depth_share: 0.125, forest_m: 25.0 }
    }
}

impl Default for Aztec {
    fn default() -> Self {
        Self {
            cell_m: 24.0,
            min_panel_m: 3.0,
            snap_m: 0.25,
            chamfer_m: 0.15,
            lean_deg: 30.0,
            tilt_deg: 5.0,
            brown: 0.2,
            roughness: 0.35,
            metallic: 0.6,
            to_star: super::direction(30.0, -50.0),
            star: 3.0,
            star_spin: 0.0,
            to_planet: super::direction(-15.0, 150.0),
            planet_deg: 20.0,
            planet: 0.12,
            exposure_stops: 0.0,
            seed: 7,
            windows: None,
            roof: None,
            unit_box: false,
        }
    }
}

impl Aztec {
    /// The hull material's `(plates, plate_finish, plate_metal)` for a region of this plate. The
    /// hull's exposure is a log curve placed for its own textures, darker than this look's paint and
    /// clipping sooner than its tone map, so the paint is scaled and the metal dulled to sit among
    /// them.
    pub fn hull_plates(&self) -> (Vec4, Vec4, Vec4) {
        (
            Vec4::new(self.cell_m, self.min_panel_m, self.snap_m, 0.0),
            Vec4::new(self.tilt_deg.to_radians(), self.brown, self.chamfer_m, self.lean_deg.to_radians()),
            Vec4::new(HULL_ROUGHNESS, HULL_METALLIC, HULL_PAINT, 0.0),
        )
    }

    /// Offices on the sides under a roof of glass hexes.
    pub fn habitat() -> Self {
        Self { windows: Some(Windows::default()), roof: Some(Roof::default()), ..default() }
    }

    pub fn material(&self) -> AztecMaterial {
        let w = self.windows.as_ref();
        AztecMaterial {
            uniforms: AztecUniform {
                grid: Vec4::new(self.cell_m, self.min_panel_m, self.snap_m, 0.0),
                chamfer: Vec4::new(self.chamfer_m, self.lean_deg.to_radians(), 0.0, 0.0),
                panel: Vec4::new(self.tilt_deg.to_radians(), self.brown, self.roughness, self.metallic),
                to_star: self.to_star.extend(self.star_spin),
                star_color: Vec3::splat(self.star).extend(0.0),
                to_planet: self.to_planet.extend(self.planet_deg.to_radians()),
                planet_color: (Vec3::new(0.55, 0.7, 1.0) * self.planet).extend(0.0),
                ambient: Vec3::splat(0.004).extend(0.0),
                finish: Vec4::new(2f32.powf(self.exposure_stops), self.seed as f32, if self.unit_box { 1.0 } else { 0.0 }, 0.0),
                deck: w.map_or(Vec4::ZERO, |w| Vec4::new(w.deck_m, w.pitch_m, w.sill_m, w.face_margin_m)),
                pane: w.map_or(Vec4::ZERO, |w| Vec4::new(w.width_m, w.height_m, w.f0, w.roughness)),
                glow: w.map_or(Vec4::ZERO, |w| w.glow.extend(w.lit)),
                day: w.map_or(Vec4::ZERO, |w| Vec4::new(w.day_glow, w.day_glass, 0.0, 0.0)),
                room: w.map_or(Vec4::ZERO, |w| Vec4::new(w.room_m, 0.0, 0.0, 0.0)),
                roof: self.roof.as_ref().map_or(Vec4::ZERO, |r| Vec4::new(r.hex_m, r.seam_m, r.rim_m, 0.0)),
                sky: self.roof.as_ref().map_or(Vec4::ZERO, |r| r.sky.extend(r.scatter)),
                outline: Vec4::ZERO,
                forest: self.roof.as_ref().map_or(Vec4::ZERO, |r| Vec4::new(r.forest_m, r.depth_share, 0.0, 0.0)),
            },
            outline: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct AztecUniform {
    grid: Vec4,
    chamfer: Vec4,
    panel: Vec4,
    to_star: Vec4,
    star_color: Vec4,
    to_planet: Vec4,
    planet_color: Vec4,
    ambient: Vec4,
    finish: Vec4,
    deck: Vec4,
    pane: Vec4,
    glow: Vec4,
    day: Vec4,
    room: Vec4,
    roof: Vec4,
    sky: Vec4,
    outline: Vec4,
    forest: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct AztecMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    uniforms: AztecUniform,
    /// The roof's outline from above, as [`super::outline`] makes it: where a ray through the
    /// glass meets the wall under the rim. A unit box knows its own, and without either the
    /// ground under the glass runs on forever.
    #[texture(1, visibility(fragment))]
    #[sampler(2, visibility(fragment))]
    outline: Option<Handle<Image>>,
}

impl AztecMaterial {
    /// `bounds` is the world XZ the map spans, `(x0, z0, x1, z1)`, and `width_m` the widest
    /// circle the outline holds, which sets how deep the forest lies.
    pub fn set_outline(&mut self, map: Handle<Image>, bounds: Vec4, width_m: f32) {
        self.outline = Some(map);
        self.uniforms.outline = bounds;
        self.uniforms.forest.z = width_m;
    }
}

impl Material for AztecMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/aztec.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/aztec.wgsl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        super::position_and_normal(descriptor, layout)
    }
}
