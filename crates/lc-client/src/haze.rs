//! The swarms' haze, marched at half resolution and added back into the sky.
//!
//! The volumetric shells cost 2.4 ms at 1280x720 from inside the Oort cloud, and four times
//! that on a Retina display. The shape is soft, so a quarter of the pixels is enough.
//!
//! A camera of its own draws only the shells, on [`HAZE_LAYER`], into a half-size float target
//! that copies the sky camera's pose, lens and viewport. A full-screen triangle in the sky pass
//! adds it back before bloom and the tone map. It sits at the stars' depth with the depth test
//! on, so a body in front still hides the haze. Rings are cheap sheets and stay in the sky pass.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{Hdr, RenderTarget};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::light::cluster::ClusterConfig;
use bevy::image::ImageSampler;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureFormat,
    TextureUsages,
};
use bevy::shader::ShaderRef;
use bevy_mesh::{Indices, PrimitiveTopology};

use crate::app::{AppState, SkyCamera, Stage};

pub const HAZE_LAYER: usize = 2;

/// Pixels of the sky's viewport per haze pixel, along each axis.
const DOWNSCALE: u32 = 2;

pub struct HazePlugin;

impl Plugin for HazePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<HazeComposite>::default())
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                follow_the_sky
                    .in_set(Stage::Scene)
                    .after(crate::app::Placed)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(OnEnter(AppState::InGame), switch(true))
            .add_systems(OnExit(AppState::InGame), switch(false));
    }
}

#[derive(Component)]
struct HazeCamera;

#[derive(Resource)]
struct Haze {
    image: Handle<Image>,
    composite: Handle<HazeComposite>,
    size: UVec2,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct HazeComposite {
    #[texture(0)]
    #[sampler(1)]
    pub haze: Handle<Image>,
}

impl Material for HazeComposite {
    fn vertex_shader() -> ShaderRef {
        "shaders/haze_composite.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/haze_composite.wgsl".into()
    }

    /// The shader returns alpha zero: `Add` is premultiplied.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers =
            vec![layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?];
        descriptor.primitive.cull_mode = None;
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

fn target(size: UVec2) -> Image {
    // Float: the haze is scene light, added before the tone map.
    let mut image =
        Image::new_target_texture(size.x.max(1), size.y.max(1), TextureFormat::Rgba16Float, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    image.sampler = ImageSampler::linear();
    image
}

/// A triangle covering the view, in clip space; the shader ignores the transform.
fn covering_triangle() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[-1.0f32, -1.0, 0.0], [3.0, -1.0, 0.0], [-1.0, 3.0, 0.0]],
        )
        .with_inserted_indices(Indices::U32(vec![0, 1, 2]))
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut composites: ResMut<Assets<HazeComposite>>,
) {
    let size = UVec2::splat(64);
    let image = images.add(target(size));
    let composite = composites.add(HazeComposite { haze: image.clone() });

    commands.spawn((
        Camera3d::default(),
        HazeCamera,
        RenderLayers::layer(HAZE_LAYER),
        RenderTarget::Image(image.clone().into()),
        Camera {
            order: -2,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            is_active: false,
            ..default()
        },
        Hdr,
        // The sky's tone map and bloom apply once, after the composite.
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        // See `app::no_lights`.
        ClusterConfig::None,
        Projection::Perspective(PerspectiveProjection::default()),
        Transform::default(),
    ));

    commands.spawn((
        Mesh3d(meshes.add(covering_triangle())),
        MeshMaterial3d(composite.clone()),
        Transform::default(),
        NoFrustumCulling,
        RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
    ));

    commands.insert_resource(Haze { image, composite, size });
}

fn switch(on: bool) -> impl FnMut(Single<&mut Camera, With<HazeCamera>>) {
    move |mut camera| camera.is_active = on
}

#[allow(clippy::type_complexity)]
fn follow_the_sky(
    sky: Single<(&Camera, &Transform, &Projection), (With<SkyCamera>, Without<HazeCamera>)>,
    haze_camera: Single<(&mut Transform, &mut Projection), With<HazeCamera>>,
    mut haze: ResMut<Haze>,
    mut images: ResMut<Assets<Image>>,
    mut composites: ResMut<Assets<HazeComposite>>,
) {
    let (sky_camera, sky_at, sky_lens) = sky.into_inner();
    let (mut at, mut lens) = haze_camera.into_inner();
    if *at != *sky_at {
        *at = *sky_at;
    }
    // Not the aspect ratio, which Bevy sets from the target.
    if let (Projection::Perspective(from), Projection::Perspective(to)) = (sky_lens, &mut *lens)
        && (from.fov, from.near, from.far) != (to.fov, to.near, to.far)
    {
        to.fov = from.fov;
        to.near = from.near;
        to.far = from.far;
    }

    let Some(viewport) = sky_camera.physical_viewport_size() else { return };
    let wanted = ((viewport + (DOWNSCALE - 1)) / DOWNSCALE).max(UVec2::ONE);
    if wanted == haze.size {
        return;
    }
    if let Some(mut image) = images.get_mut(&haze.image) {
        *image = target(wanted);
    }
    // The bind group still holds the texture that was just replaced.
    if let Some(mut composite) = composites.get_mut(&haze.composite) {
        composite.haze = haze.image.clone();
    }
    haze.size = wanted;
}
