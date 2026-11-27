//! The sky view drawn with the game's starfield material: a camera at the sky's eye renders the
//! baked stars through `shaders/starfield.wgsl` into an image, and the view shows the image. The
//! dot view stays for coloring by attribute.
//!
//! The view asks for a frame by filling [`SkyRequest`] during the egui pass; [`drive`] then
//! sizes the image, points the camera and rebuilds the stars for the next frame. The image the
//! view shows is therefore a frame behind, which a sky that moves only when dragged hides.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor, TextureViewDimension};
use bevy::camera::Hdr;
use bevy_egui::{EguiTextureHandle, EguiUserTextures, egui};
use em_render::relativistic_starfield_material::{
    LOG_T_MIN, LUT_SAMPLES, RelativisticStarfieldMaterial, RelativisticStarfieldUniform, StarPoint, band_columns, band_lut, build_mesh,
    lut_scale,
};
use em_render::render_space::sim_to_render;
use em_spectra::point::{DISTANT, LUMA, POINT_STOPS};
use glam::DVec3;

use crate::doc::Doc;

/// What the view wants drawn this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct SkyRequest {
    /// Physical pixels.
    pub size: [u32; 2],
    /// Physical pixels per logical one: the style's sizes are logical, as the game's are.
    pub scale: f32,
    pub eye_ly: DVec3,
    /// The star the sky is seen from, left out of it: from inside its system it would fill the
    /// screen.
    pub own: Option<usize>,
    /// Radians: about +Z from +X, and above the XY plane.
    pub yaw: f64,
    pub pitch: f64,
    pub fov: f64,
    /// Stops above the automatic exposure.
    pub exposure: f64,
    pub generation: u64,
    /// What the toolbar solos, which picks the stars drawn; with `shown` it keys the rebuild.
    pub solo: Option<crate::compare::Solo>,
    /// Per baked star, whether solo keeps it.
    pub shown: Vec<bool>,
}

#[derive(Component)]
pub struct SkyCamera;

#[derive(Resource)]
pub struct SkyScene {
    image: Handle<Image>,
    mesh: Handle<Mesh>,
    material: Handle<RelativisticStarfieldMaterial>,
    /// The request the mesh and exposure were built for, apart from where the camera looks.
    built: Option<(u64, DVec3, Option<usize>, Option<crate::compare::Solo>)>,
    reference: f32,
}

/// Spawns the camera, the image it renders into and the star mesh, and returns the egui id the
/// view draws the image with.
pub fn setup(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<RelativisticStarfieldMaterial>,
    egui_textures: &mut EguiUserTextures,
) -> egui::TextureId {
    let mut target = Image::new_target_texture(16, 16, TextureFormat::Rgba8UnormSrgb, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let image = images.add(target);
    let texture = egui_textures.add_image(EguiTextureHandle::Strong(image.clone()));
    // The distant style draws no corona, but the material binds its two cubemaps regardless.
    let blank = images.add(blank_cube());
    let material = materials.add(RelativisticStarfieldMaterial {
        uniforms: RelativisticStarfieldUniform::default(),
        band_lut: images.add(band_lut()),
        corona_filaments: blank.clone(),
        corona_reach: blank,
    });
    let mesh = meshes.add(build_mesh(&[], DVec3::ZERO));
    commands.spawn((
        Camera3d::default(),
        Camera { order: -1, is_active: false, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
        RenderTarget::Image(ImageRenderTarget { handle: image.clone(), scale_factor: 1.0 }),
        Projection::Perspective(PerspectiveProjection { near: 1e-3, far: 1e9, ..default() }),
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
        SkyCamera,
    ));
    commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), NoFrustumCulling));
    commands.insert_resource(SkyScene { image, mesh, material, built: None, reference: 1.0 });
    texture
}

fn blank_cube() -> Image {
    let mut cube = Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 6 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    cube.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    cube
}

/// Readies the next frame for `request`, or stops the camera when there is none.
#[allow(clippy::too_many_arguments)]
pub fn drive(
    request: Option<&SkyRequest>,
    doc: &Doc,
    scene: &mut SkyScene,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<RelativisticStarfieldMaterial>,
    camera: (&mut Camera, &mut Transform, &mut Projection),
) {
    let (camera, transform, projection) = camera;
    // Bloom scales its chain to a fixed height, so a view laid out a pixel tall, as one is on
    // its first frame, asks for a texture over a hundred thousand pixels wide and fails.
    let request = request.filter(|r| r.size.iter().all(|&n| n >= 16));
    camera.is_active = request.is_some();
    let Some(r) = request else { return };
    let size = r.size.map(|n| n.min(8192));
    if let Some(mut image) = images.get_mut(&scene.image)
        && (image.width(), image.height()) != (size[0], size[1])
    {
        image.resize(Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 });
    }

    let mapping = em_spectra::presets::natural();
    let key = (r.generation, r.eye_ly, r.own, r.solo);
    if scene.built != Some(key) {
        let stars = doc.stars();
        let points: Vec<StarPoint> = stars
            .iter()
            .enumerate()
            .filter(|(i, s)| Some(*i) != r.own && r.shown.get(*i).copied().unwrap_or(true) && s.attributes.luminosity_solar > 0.0 && s.attributes.teff_k > 0.0)
            .map(|(_, s)| StarPoint {
                position_ly: s.position_ly,
                teff_k: s.attributes.teff_k as f32,
                radius_m: (s.attributes.radius_solar * 6.957e8) as f32,
                // Mixed first, as the game's are: the id's top bits are its cell, which every
                // star in the cell shares.
                seed: (lc_world::rng::mix(s.id.get()) >> 40) as f32 * 1.0e-3,
                warm: [0.0; 4],
            })
            .collect();
        scene.reference = reference(&points, r.eye_ly, &mapping);
        if let Some(mut mesh) = meshes.get_mut(&scene.mesh) {
            *mesh = build_mesh(&points, r.eye_ly);
        }
        scene.built = Some(key);
    }

    let (sy, cy) = r.yaw.sin_cos();
    let (sp, cp) = r.pitch.sin_cos();
    let look = sim_to_render(DVec3::new(cp * cy, cp * sy, sp)).as_vec3();
    *transform = Transform::IDENTITY.looking_to(look, Vec3::Y);
    if let Projection::Perspective(p) = projection {
        p.fov = r.fov as f32;
    }
    // The style's sizes are logical pixels and the shader rescales by physical ones, as the
    // game's sky does, so a star is the same size here as there on any display.
    let rad_per_px = 2.0 * (r.fov as f32 * 0.5).tan() / size[1] as f32;
    let rad_per_logical_px = rad_per_px * r.scale.max(1.0);
    if let Some(mut material) = materials.get_mut(&scene.material) {
        material.uniforms = RelativisticStarfieldUniform {
            band_to_display: band_columns(&mapping),
            reference: scene.reference * 2f32.powf(-(r.exposure as f32)),
            point_stops: POINT_STOPS,
            min_radius_rad: DISTANT.min_px * rad_per_logical_px,
            max_radius_rad: DISTANT.max_px * rad_per_logical_px,
            glow_radius_gain: DISTANT.glow_radius_gain,
            brightness: DISTANT.brightness,
            overflow_gain: DISTANT.overflow_gain,
            halo_gain: DISTANT.halo_gain,
            halo_falloff: DISTANT.halo_falloff,
            corona_strength: 0.0,
            log_t_min: LOG_T_MIN,
            log_t_scale: lut_scale(),
            lut_samples: LUT_SAMPLES as f32,
            drawn_rad_per_px: rad_per_px,
            drawn_exposure: bevy::camera::Exposure::default().exposure(),
            ..default()
        };
    }
}

/// The luminance at the top of the window: the 99.5th percentile of what the stars deliver to
/// `eye`, so the brightest few overflow into glare as the game's metering leaves them.
fn reference(points: &[StarPoint], eye: DVec3, mapping: &em_spectra::BandMapping) -> f32 {
    let mut seen: Vec<f64> = points
        .iter()
        .map(|p| {
            let d_m = p.position_ly.distance(eye).max(1e-6) * 9.460_730_472_580_8e15;
            let solid_angle = std::f64::consts::PI * (f64::from(p.radius_m) / d_m).powi(2);
            let rgb = mapping.apply_f64(&em_spectra::blackbody::per_band(f64::from(p.teff_k)));
            rgb.iter().zip(LUMA).map(|(c, w)| c * f64::from(w)).sum::<f64>() * solid_angle
        })
        .filter(|l| *l > 0.0)
        .collect();
    if seen.is_empty() {
        return 1.0;
    }
    seen.sort_by(f64::total_cmp);
    seen[((seen.len() as f64 * 0.995) as usize).min(seen.len() - 1)] as f32
}
