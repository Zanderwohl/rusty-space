//! The map's camera, its render target, and the frame it draws. The entities are
//! [`crate::map_scene`]'s.
//!
//! A second `Camera3d` on its own [`MAP_LAYER`], rendering into an [`Image`] that egui shows.
//!
//! Transforms are camera-relative, so an entity belongs to exactly one camera: the whole view
//! and the corner square share one image because two aimed views would need two sets of
//! entities.
//!
//! No `Hdr`, bloom or tone map. The sky's camera is metered for a photograph and a diagram
//! needs a different range — see [`LINE_COLOR_SCALE`].

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureFormat, TextureUsages};
use bevy_egui::EguiUserTextures;
use em_map::{ItemKey, MapFrame, MapSnapshot, Placement, compose};

use glam::DVec3;

use crate::app::{Game, Stage, Ui};
use crate::map_scene::{Form, Scene, Shapes, Viewport, form_of, render};
use crate::map_source::Source;

/// The map's own layer. The sky keeps layer 0, so neither camera sees the other's entities.
pub const MAP_LAYER: usize = 1;

/// What the target starts at, before a surface has asked for a size.
const INITIAL_SIDE: u32 = 512;

/// egui reports a zero rect on the frame a window opens, and a zero-sized texture is a wgpu
/// validation failure. The ceiling bounds a runaway resize drag on a dense display.
const MIN_SIDE: u32 = 64;
const MAX_SIDE: u32 = 4096;

/// The near and far planes, as multiples of the stand-off: wide enough for a ship beside the
/// camera and the outermost ring at once, and no wider.
const NEAR_FRACTION: f32 = 1.0e-4;
const FAR_MULTIPLE: f32 = 1.0e6;

/// The map camera's vertical field of view. Named because the panel casts the cursor's ray
/// with it, and a camera and a cursor that disagree put the anchor away from the pointer.
pub const MAP_FOV: f32 = std::f32::consts::FRAC_PI_4;

/// The camera the map is drawn for.
#[derive(Component)]
pub struct MapCamera;

#[derive(Resource)]
pub struct Map {
    pub image: Handle<Image>,
    pub texture: Option<bevy_egui::egui::TextureId>,
    /// What the target currently is.
    pub size: UVec2,
    /// What the surface drawing it asked for, last frame. egui runs after the scene stage, so
    /// a resize lands one frame late and that frame shows the previous texture stretched.
    pub wanted: UVec2,
    /// Whether anything is showing the map. Nothing is drawn when nothing is looking.
    pub shown: bool,
    pub snapshot: MapSnapshot,
    /// What each item of the snapshot is, in the terms the rest of the interface selects
    /// things in. See [`crate::map_source::Picture`].
    pub subjects: std::collections::HashMap<ItemKey, crate::pick::Subject>,
    /// Which item holds the ship, when the snapshot has one. Worked out beside the snapshot
    /// because that is where the session is.
    pub primary: Option<ItemKey>,
    pub frame: Option<MapFrame>,
    pub(crate) shapes: Shapes,
    pub(crate) scene: Scene,
    /// Whether this frame builds and renders the map. See [`pace`].
    pub(crate) due: bool,
    /// Real seconds at the last frame that did, and the controls it was drawn with.
    drawn_at_s: f64,
    drawn_for: Option<Controls>,
}

/// What a player moves the map's camera with. A change is drawn at once, however the map is shown.
type Controls = (f64, f64, f64, em_map::Plane, crate::ui::MapFocus, Source);

fn controls(view: &crate::ui::MapView) -> Controls {
    (view.orbit.azimuth, view.orbit.elevation, view.orbit.log_distance_m, view.plane, view.focus, view.source)
}

/// How often the corner thumbnail is drawn while nobody is moving it. The texture holds the
/// last picture between, and the labels are laid out from the same frame, so the two agree.
const THUMBNAIL_PERIOD_S: f64 = 0.1;

impl Map {
    /// Where the reference plane is anchored: the observer, or the camera's focus when a
    /// snapshot has nobody in it.
    pub fn plane_origin_ly(&self, focus_ly: DVec3) -> DVec3 {
        self.snapshot.observer().map_or(focus_ly, |o| o.position_ly)
    }

    fn viewport(&self, fov_y: f32) -> Viewport {
        Viewport::new(self.size.y, fov_y)
    }

    /// How big a mark is, in texture pixels. A label has to clear it, after scaling by the
    /// points that texture is shown at.
    pub fn symbol_px(&self) -> f32 {
        self.viewport(MAP_FOV).point_px
    }

    /// The radius a placement is drawn at, in texture pixels. Not the nominal mark size: a
    /// resolved body is a sphere at its own angular size and only an unresolved one falls back
    /// to the symbol. Picking reads this so that it agrees with the picture. See [`Form`].
    pub fn drawn_radius_px(&self, placement: &Placement) -> f32 {
        let view = self.viewport(MAP_FOV);
        match form_of(placement, view) {
            Form::Sphere => placement.angular_radius / view.rad_per_px.max(f32::MIN_POSITIVE),
            Form::Circle | Form::Dot => view.mark_px(placement) * 0.5,
        }
    }

    /// Points of a surface per pixel of the texture. They differ on a display that scales, and
    /// on the frame after a resize.
    pub fn points_per_pixel(&self, surface_height: f32) -> f32 {
        match self.size.y {
            0 => 1.0,
            height => surface_height / height as f32,
        }
    }
}

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::map_line::MapLinePlugin)
            .init_resource::<crate::beliefs::Beliefs>()
            .add_systems(Startup, setup)
            // **After the scene the snapshot is built from.** Taken in `Stage::Act`, it held
            // the previous frame's eye while the contacts in it were this frame's, so this
            // ship's own mark trailed one frame behind everything around it — a jitter
            // whenever the ship was under way.
            .add_systems(
                Update,
                (pace, survey, resize, place, crate::map_scene::lay, switch_camera)
                    .chain()
                    .in_set(Stage::Scene)
                    .after(crate::app::Placed),
            )
            .add_systems(OnExit(crate::app::AppState::InGame), hide);
    }
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut textures: ResMut<EguiUserTextures>,
) {
    let image = images.add(target_image(UVec2::splat(INITIAL_SIDE)));
    let texture = textures.add_image(bevy_egui::EguiTextureHandle::Strong(image.clone()));
    let map_image = image.clone();

    commands.insert_resource(Map {
        texture: Some(texture),
        size: UVec2::splat(INITIAL_SIDE),
        wanted: UVec2::splat(INITIAL_SIDE),
        shown: false,
        snapshot: MapSnapshot::observed(0.0, Vec::new()),
        subjects: std::collections::HashMap::new(),
        primary: None,
        frame: None,
        shapes: Shapes::new(&mut meshes),
        scene: Scene::default(),
        due: false,
        drawn_at_s: f64::NEG_INFINITY,
        drawn_for: None,
        image,
    });

    commands.spawn((
        Camera3d::default(),
        MapCamera,
        RenderLayers::layer(MAP_LAYER),
        // A component of its own in Bevy 0.19, not a field on `Camera`. Left off, this camera
        // renders over the primary window on an empty layer and clears everything to black,
        // with nothing in the log to say why.
        RenderTarget::Image(map_image.into()),
        Camera {
            // Before the window camera, whose frame shows what this one drew.
            order: -1,
            // Until something shows it: see `switch_camera`.
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection { fov: MAP_FOV, ..default() }),
        // See the module doc.
        Tonemapping::None,
        Transform::default(),
    ));
}

/// Whether this frame draws the map: always as the main view, and in the corner only every
/// [`THUMBNAIL_PERIOD_S`] unless the player is moving it.
fn pace(time: Res<Time<Real>>, ui: Res<Ui>, mut map: ResMut<Map>) {
    let now_s = time.elapsed_secs_f64();
    let resized = map.size != map.wanted.clamp(UVec2::splat(MIN_SIDE), UVec2::splat(MAX_SIDE));
    let stale = map.frame.is_none() || resized || map.drawn_for != Some(controls(&ui.map));
    map.due = map.shown && due(ui.view == crate::ui::ViewMode::Map, stale, now_s - map.drawn_at_s);
    if map.due {
        map.drawn_at_s = now_s;
    }
}

fn due(main_view: bool, stale: bool, since_s: f64) -> bool {
    main_view || stale || since_s >= THUMBNAIL_PERIOD_S
}

/// Render the map only on a frame that draws it. In the menu and the loading screen nothing
/// shows it, and between the thumbnail's frames its texture keeps the last picture.
///
/// The controls are taken here rather than in [`pace`], after [`place`] has turned the camera
/// with a tracked frame: taken before, that turn read as a player's and drew every frame.
fn switch_camera(ui: Res<Ui>, mut map: ResMut<Map>, mut camera: Single<&mut Camera, With<MapCamera>>) {
    if map.due {
        map.drawn_for = Some(controls(&ui.map));
    }
    if camera.is_active != map.due {
        camera.is_active = map.due;
    }
}

/// Only the game's interface pass sets `shown`, so leaving the game has to clear it.
fn hide(mut map: ResMut<Map>) {
    map.shown = false;
}

pub(crate) fn target_image(size: UVec2) -> Image {
    let mut image = Image::new_target_texture(
        size.x.max(MIN_SIDE),
        size.y.max(MIN_SIDE),
        // 8-bit sRGB, so egui samples it and gets back what was drawn. A float target is
        // stored linear and comes out of `ui.image` looking like a shading bug.
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    // `Image::resize` copies the old contents forward so a resize does not flash, and
    // `new_target_texture` leaves out the COPY_SRC that needs. Without it the first resize is
    // a wgpu validation failure that ends the process.
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image
}

/// Reallocate the target when the surface drawing it has changed size, and not otherwise:
/// resizing an `Image` asset reallocates a GPU texture.
fn resize(mut map: ResMut<Map>, mut images: ResMut<Assets<Image>>) {
    let wanted = map.wanted.clamp(UVec2::splat(MIN_SIDE), UVec2::splat(MAX_SIDE));
    if wanted == map.size {
        return;
    }
    let Some(mut image) = images.get_mut(&map.image) else { return };
    image.resize(Extent3d { width: wanted.x, height: wanted.y, ..default() });
    map.size = wanted;
}

/// Build this frame's snapshot from whichever source the interface is showing.
fn survey(
    game: Res<Game>,
    mut ui: ResMut<Ui>,
    uplink: Res<crate::uplink::Uplink>,
    eye: Res<crate::hull::Eye>,
    mut map: ResMut<Map>,
    mut beliefs: ResMut<crate::beliefs::Beliefs>,
) {
    if !map.due {
        return;
    }
    let held = beliefs.held(&game.0);
    // Before anything is composed: the plane the camera's angles are measured against is the
    // one this craft has solved for the system it is in, and a ship that crossed to another
    // star is in another one. Truth's pole is deliberately not read here -- what the map draws
    // is what the crew worked out. See `lightcone/docs/25-system-knowledge.md`.
    ui.map.system_plane = held.plane;
    let picture = match ui.map.source {
        Source::Observed => crate::map_source::observed(&game.0, &uplink, eye.at_ly, held),
        #[cfg(feature = "godview")]
        Source::God => crate::map_source::coordinate(&game.0, &uplink, eye.at_ly),
    };
    map.snapshot = picture.snapshot;
    map.subjects = picture.subjects;
    map.primary = crate::map_source::primary(&game.0);
}

#[allow(clippy::too_many_arguments)]
fn place(
    mut map: ResMut<Map>,
    mut ui: ResMut<Ui>,
    camera: Single<(&mut Transform, &mut Projection), With<MapCamera>>,
) {
    let (mut transform, mut projection) = camera.into_inner();
    if !map.shown {
        map.frame = None;
        return;
    }
    if !map.due {
        return;
    }

    // Follow the selection, not a remembered position: Saturn moves. Written back rather than
    // applied to a copy, because every action that moves the camera starts from the
    // interface's own `focus_ly`, and a stale one makes the first frame of a drag jump.
    follow(ui.map.focus, &map.snapshot, map.primary, &mut ui.map.orbit.focus_ly);
    // After the follow: the line is measured from where the camera is now looking.
    spin(&mut ui.map, &map.snapshot, map.primary);
    let view = ui.map;
    let meters_per_unit = crate::view::ScaleTier::for_distance(view.orbit.distance_m())
        .meters_per_unit();
    let frame = compose(&map.snapshot, &view.orbit, view.datum(), meters_per_unit);

    // The depth range is written from the stand-off every frame rather than fixed. The sky's
    // camera spans 1e-10 to 1e9 because it has to cover everything at once; the map's distance
    // is a piece of state, so it can be exact — and reversed depth makes the span cheap.
    let standoff = (view.orbit.distance_m() / meters_per_unit) as f32;
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.near = (standoff * NEAR_FRACTION).max(f32::MIN_POSITIVE);
        perspective.far = standoff * FAR_MULTIPLE;
    }

    // The eye is the render origin, and the map looks back at its focus.
    let (forward, up) = view.orbit.orientation(view.datum());
    transform.translation = Vec3::ZERO;
    transform.look_to(render(forward), render(up));

    let fov_y = match &*projection {
        Projection::Perspective(perspective) => perspective.fov,
        _ => MAP_FOV,
    };
    let view = map.viewport(fov_y);
    map.scene.view = Some((view, standoff));
    map.frame = Some(frame);
}

/// Put `focus_ly` where the focus says to look, and say whether it moved. Pans and zooms start
/// from this value, so a stale one is a jump.
pub fn follow(
    focus: crate::ui::MapFocus,
    snapshot: &MapSnapshot,
    primary: Option<ItemKey>,
    focus_ly: &mut DVec3,
) -> bool {
    match focus_position(focus, snapshot, primary) {
        Some(at) if at != *focus_ly => {
            *focus_ly = at;
            true
        }
        _ => false,
    }
}

/// Where the camera should look, or `None` to leave it. A key no longer in the snapshot also
/// leaves it, so a body going out of range stops the camera rather than moving it to nowhere.
pub fn focus_position(
    focus: crate::ui::MapFocus,
    snapshot: &MapSnapshot,
    primary: Option<ItemKey>,
) -> Option<DVec3> {
    let at = |key| snapshot.item(key).map(|i| i.position_ly);
    match focus {
        crate::ui::MapFocus::Free => None,
        crate::ui::MapFocus::Observer => snapshot.observer().map(|o| o.position_ly),
        crate::ui::MapFocus::Primary(_) => primary.and_then(at),
        crate::ui::MapFocus::Item(key) => at(key),
    }
}

/// Turn the camera with the reference line, when the focus asks for it, and say whether it
/// moved.
///
/// The line runs from the primary's center to the ship's, and the local frame holds the camera
/// against it: the ship keeps its place on screen and the rest of the system goes round. What
/// is written is the *change* in the line's bearing, so the camera's azimuth stays the one
/// number a drag, a ray and a label are all measured in.
pub fn spin(view: &mut crate::ui::MapView, snapshot: &MapSnapshot, primary: Option<ItemKey>)
    -> bool {
    let line = match view.focus {
        crate::ui::MapFocus::Primary(crate::ui::Frame::Local) => reference_line(snapshot, primary),
        _ => None,
    };
    let Some(bearing) = line.and_then(|line| view.datum().bearing(line)) else {
        // Nothing to hold onto: the camera stays where it is and starts again from whatever
        // the line reads next.
        view.bearing = None;
        return false;
    };
    let turned = match view.bearing {
        // The difference, with no mending at the seam: an azimuth is wrapped into its own
        // circle, so a step that reads as a whole turn backwards lands in the same place as
        // the hair's turn it really is.
        Some(was) => {
            view.orbit.turn(bearing - was, 0.0);
            bearing != was
        }
        None => false,
    };
    view.bearing = Some(bearing);
    turned
}

/// The primary's center to the ship's, in light-years, or `None` when the snapshot is missing
/// either end.
fn reference_line(snapshot: &MapSnapshot, primary: Option<ItemKey>) -> Option<DVec3> {
    let ship = snapshot.observer()?.position_ly;
    let at = snapshot.item(primary?)?.position_ly;
    Some(ship - at)
}

#[cfg(test)]
mod tests {
    use em_render::body_material::BASE_TUBE_RADIUS;

    use super::*;
    use crate::map_scene::*;
    use crate::map_spread::cap_transform;

    /// The main view draws every frame; the corner every tenth of a second, or at once when
    /// something about it has changed.
    #[test]
    fn the_thumbnail_is_drawn_ten_times_a_second_unless_moved() {
        assert!(due(true, false, 0.0), "the main view waits for nothing");
        assert!(!due(false, false, 0.05), "the corner redrew early");
        assert!(due(false, false, THUMBNAIL_PERIOD_S), "the corner stopped");
        assert!(due(false, true, 0.0), "a moved camera waited");
    }

    /// A cap reads as square to its bar from any angle, centered on the end it closes and the
    /// same size on screen however far off it is.
    #[test]
    fn an_error_bar_cap_is_square_to_its_bar_and_to_the_eye() {
        let (near, far) = (Vec3::new(10.0, 0.0, 3.0), Vec3::new(12.0, 1.0, 2.0));
        let rad_per_px = 1.0e-3;
        let cap = cap_transform(near, far, rad_per_px);
        let across = cap.rotation * Vec3::Y;
        let (end, other) = (render(near.as_dvec3()), render(far.as_dvec3()));
        assert!(across.dot((other - end).normalize()).abs() < 1.0e-5, "not square to the bar");
        assert!(across.dot(end.normalize()).abs() < 1.0e-5, "not square to the line of sight");
        let middle = cap.translation + across * (0.5 * cap.scale.y);
        assert!(middle.distance(end) < 1.0e-4, "not centered on its end");
        assert!((cap.scale.y - end.length() * rad_per_px * SPREAD_CAP_PX).abs() < 1.0e-5);
    }
    use crate::map_line::tube_radius;

    use crate::ui::{Frame, MapFocus};
    use em_map::{ItemKey, ItemKind, MapItem, MapSnapshot};
    use glam::DVec3;

    fn snapshot() -> MapSnapshot {
        MapSnapshot::observed(0.0, vec![
            MapItem::body(ItemKey::from_name("observer"), "Anonymous Ship", ItemKind::Observer,
                DVec3::new(1.0, 2.0, 3.0), 100.0, DVec3::Z),
            MapItem::body(ItemKey::from_id("star", 7), "Sol", ItemKind::Star,
                DVec3::new(4.0, 5.0, 6.0), 7.0e8, DVec3::Z),
        ])
    }

    /// The observer needs a focus state of its own. `None` has to mean "leave the camera
    /// alone" for a pan, so the ship cannot share it.
    #[test]
    fn centering_on_the_ship_finds_the_ship() {
        let snapshot = snapshot();
        assert_eq!(
            focus_position(MapFocus::Observer, &snapshot, None),
            Some(DVec3::new(1.0, 2.0, 3.0)),
        );
        assert_eq!(
            focus_position(MapFocus::Item(ItemKey::from_id("star", 7)), &snapshot, None),
            Some(DVec3::new(4.0, 5.0, 6.0)),
        );
    }

    /// A pan starts from the interface's own `focus_ly`, so `place` has to write the followed
    /// position back into it and not into a copy.
    #[test]
    fn a_pan_begins_where_the_camera_actually_is() {
        let snapshot = snapshot();
        let ship = DVec3::new(1.0, 2.0, 3.0);
        let mut orbit = em_map::Orbit::framing(DVec3::ZERO, em_map::snapshot::M_PER_AU * 40.0);
        assert_eq!(orbit.focus_ly, DVec3::ZERO, "premise: it starts at the origin");

        follow(MapFocus::Observer, &snapshot, None, &mut orbit.focus_ly);
        assert_eq!(orbit.focus_ly, ship, "following did not reach the interface's copy");

        // Now the drag. A small pan has to leave the camera near the ship, not near zero.
        orbit.pan(em_map::Plane::System.about(DVec3::Z), 0.05, 0.0);
        let moved = orbit.focus_ly.distance(ship);
        assert!(moved > 0.0, "the pan moved nothing");
        assert!(
            moved < 0.25 * ship.length(),
            "the pan threw the camera {moved} ly from the ship, which is the jump",
        );
    }

    /// Following writes only on a change, so a resource half the interface watches is not
    /// marked dirty every frame.
    #[test]
    fn following_the_same_place_twice_writes_once() {
        let snapshot = snapshot();
        let mut at = DVec3::ZERO;
        assert!(follow(MapFocus::Observer, &snapshot, None, &mut at), "the first call should move it");
        assert!(!follow(MapFocus::Observer, &snapshot, None, &mut at), "the second should not");
        assert!(!follow(MapFocus::Free, &snapshot, None, &mut at), "free never moves it");
    }

    /// **Every mark is drawn in the palette, and the palette has no white in it.** A color
    /// written straight into a match arm is one the interface cannot re-theme and one nothing
    /// else agrees with.
    #[test]
    fn nothing_is_drawn_outside_the_palette() {
        let palette = [
            em_ui::vfd::TEXT,
            em_ui::vfd::TEXT_DIM,
            em_ui::vfd::BUTTON_BORDER,
            em_ui::vfd::AMBER,
        ];
        for kind in [
            ItemKind::Star,
            ItemKind::Planet,
            ItemKind::Moon,
            ItemKind::Minor,
            ItemKind::Population,
            ItemKind::Ship,
            ItemKind::Station,
            ItemKind::Observer,
        ] {
            assert!(palette.contains(&color_of(kind)), "{kind:?} is drawn off the palette");
        }
    }

    /// **The primary is a mode, not the body it resolves to today.** Centering on it and
    /// centering on Earth are the same picture while the ship is at Earth, and different
    /// pictures the moment it is not.
    #[test]
    fn the_primary_is_whatever_the_map_is_told_holds_the_ship() {
        let snapshot = snapshot();
        let star = ItemKey::from_id("star", 7);
        assert_eq!(
            focus_position(MapFocus::Primary(Frame::Fixed), &snapshot, Some(star)),
            Some(DVec3::new(4.0, 5.0, 6.0)),
        );
        // Nothing holding it, and a body that is no longer in the snapshot: both leave the
        // camera where it is rather than moving it to nowhere.
        assert_eq!(focus_position(MapFocus::Primary(Frame::Fixed), &snapshot, None), None);
        assert_eq!(focus_position(MapFocus::Primary(Frame::Fixed), &snapshot, Some(ItemKey(999))), None);
    }

    /// A ship at `bearing` radians round its primary, a light-year out.
    fn ship_at(bearing: f64) -> MapSnapshot {
        let star = DVec3::new(4.0, 5.0, 6.0);
        MapSnapshot::observed(0.0, vec![
            MapItem::body(ItemKey::from_name("observer"), "Anonymous Ship", ItemKind::Observer,
                star + DVec3::new(bearing.cos(), bearing.sin(), 0.0), 100.0, DVec3::Z),
            MapItem::body(ItemKey::from_id("star", 7), "Sol", ItemKind::Star, star, 7.0e8,
                DVec3::Z),
        ])
    }

    /// The shortest turn from `a` to `b`. An azimuth is stored wrapped into a circle, so
    /// subtracting two of them is not the turn between them.
    fn apart(a: f64, b: f64) -> f64 {
        let by = (b - a).rem_euclid(std::f64::consts::TAU);
        match by > std::f64::consts::PI {
            true => by - std::f64::consts::TAU,
            false => by,
        }
    }

    fn locked_on(frame: Frame) -> crate::ui::MapView {
        crate::ui::MapView {
            focus: MapFocus::Primary(frame),
            plane: em_map::Plane::System,
            // Solved, and solved as `+Z`, because these ships are placed in the `xy` plane: a
            // quarter of an orbit is a quarter turn of the camera only when the orbit lies in
            // the plane the camera is angled against. A craft that has solved nothing gets the
            // galactic frame, which this motion is not in.
            system_plane: lc_world::knowledge::SystemPlane::Known {
                pole: DVec3::Z,
                sigma_rad: 0.0,
                zero: DVec3::X,
            },
            ..Default::default()
        }
    }

    /// **The local frame holds the camera against the reference line.** A quarter of an orbit
    /// turns the camera a quarter, so the ship keeps its place on screen and the system goes
    /// round it. The fixed frame turns nothing and the ship is what moves.
    #[test]
    fn the_local_frame_turns_with_the_ship() {
        use std::f64::consts::FRAC_PI_2;
        let star = Some(ItemKey::from_id("star", 7));
        for (frame, expected) in [(Frame::Local, FRAC_PI_2), (Frame::Fixed, 0.0)] {
            let mut view = locked_on(frame);
            let was = view.orbit.azimuth;
            // The first call has nothing to measure against, so it turns nothing.
            assert!(!spin(&mut view, &ship_at(0.0), star), "{frame:?} turned on the first frame");
            assert_eq!(view.orbit.azimuth, was);

            spin(&mut view, &ship_at(FRAC_PI_2), star);
            let by = apart(was, view.orbit.azimuth);
            assert!((by - expected).abs() < 1.0e-9, "{frame:?} turned by {by}, wanted {expected}");
        }
    }

    /// **A whole orbit brings the camera back to where it started.** The step from just under
    /// +pi to just over it reads as a turn backwards round the whole circle, and the camera
    /// lands in the same place either way only because it is turned *by* the change rather
    /// than set *to* the bearing.
    #[test]
    fn a_whole_orbit_brings_the_camera_back() {
        use std::f64::consts::TAU;
        let star = Some(ItemKey::from_id("star", 7));
        for way in [1.0, -1.0] {
            let mut view = locked_on(Frame::Local);
            view.orbit.turn(1.234, 0.0);
            spin(&mut view, &ship_at(0.0), star);
            let start = view.orbit.azimuth;
            for step in 1..=16 {
                spin(&mut view, &ship_at(way * step as f64 * TAU / 16.0), star);
            }
            let off = apart(start, view.orbit.azimuth);
            assert!(off.abs() < 1.0e-9, "an orbit {way} came back {off} out");
        }
    }

    /// Leaving the frame leaves the camera where it is, and coming back starts again from
    /// wherever the line is then — neither is a jump.
    #[test]
    fn a_frame_is_left_and_entered_without_a_jump() {
        let star = Some(ItemKey::from_id("star", 7));
        let mut view = locked_on(Frame::Local);
        spin(&mut view, &ship_at(0.0), star);
        spin(&mut view, &ship_at(1.0), star);
        let held = view.orbit.azimuth;

        view.focus = MapFocus::Primary(Frame::Fixed);
        assert!(!spin(&mut view, &ship_at(2.0), star), "the fixed frame turned the camera");
        assert_eq!(view.orbit.azimuth, held);
        assert_eq!(view.bearing, None, "nothing is being tracked");

        // Back, from a line that has moved a long way since.
        view.focus = MapFocus::Primary(Frame::Local);
        assert!(!spin(&mut view, &ship_at(3.0), star), "coming back turned the camera");
        assert_eq!(view.orbit.azimuth, held);
    }

    /// Nothing to hold onto: a snapshot without one end of the line leaves the camera alone
    /// rather than turning it to an arbitrary bearing.
    #[test]
    fn a_line_with_one_end_missing_turns_nothing() {
        let mut view = locked_on(Frame::Local);
        spin(&mut view, &ship_at(0.0), Some(ItemKey::from_id("star", 7)));
        let held = view.orbit.azimuth;
        assert!(!spin(&mut view, &ship_at(1.0), None), "no primary, no line");
        assert!(!spin(&mut view, &MapSnapshot::observed(0.0, Vec::new()),
            Some(ItemKey::from_id("star", 7))), "no ship, no line");
        assert_eq!(view.orbit.azimuth, held);
        assert_eq!(view.bearing, None);
    }

    /// A pan has to be able to leave the camera where it is.
    #[test]
    fn a_free_camera_is_left_where_it_was_put() {
        assert_eq!(focus_position(MapFocus::Free, &snapshot(), None), None);
    }

    /// A map opens on the observer, not on the world origin.
    #[test]
    fn a_map_opens_on_the_ship() {
        assert_eq!(crate::ui::MapView::default().focus, MapFocus::Observer);
    }

    /// Following something out of range stops following it rather than moving the view.
    #[test]
    fn following_something_that_is_gone_holds_still() {
        assert_eq!(focus_position(MapFocus::Item(ItemKey(999)), &snapshot(), None), None);
        assert_eq!(
            focus_position(MapFocus::Observer, &MapSnapshot::observed(0.0, Vec::new()), None),
            None,
        );
    }

    /// No tube can contain the camera, at any scale or distance. The inside of a tube is opaque.
    #[test]
    fn no_tube_can_reach_the_camera() {
        for height in [64.0f32, 410.0, 2160.0] {
            let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / height;
            for scale in [1.0e-6f32, 1.0, 1.0e5] {
                for distance in [1.0e-6f32, 1.0, 6.33e2, 1.81e5] {
                    for (fraction, width) in [(LINE_TUBE_FRACTION, SCALE_PX),
                        (LINE_TUBE_FRACTION, LINE_PX), (SPHERE_TUBE_FRACTION, LINE_PX)]
                    {
                        let world =
                            scale * tube_radius(scale, rad_per_px, distance, fraction, width);
                        assert!(
                            world < distance,
                            "a tube {world:e} wide at {distance:e}, scale {scale:e}, {height} px",
                        );
                    }
                }
            }
        }
    }

    /// A 410-pixel surface, which is about what the panel's map area is.
    fn viewport() -> Viewport {
        Viewport::new(410, std::f32::consts::FRAC_PI_4)
    }

    fn body_at(distance: f32, radius: f32) -> Placement {
        kind_at(ItemKind::Planet, distance, radius)
    }

    fn kind_at(kind: ItemKind, distance: f32, radius: f32) -> Placement {
        Placement {
            key: ItemKey::from_name("a body"),
            kind,
            weight: 0.0,
            symbol_scale: 1.0,
            label: "a body".into(),
            at: glam::Vec3::new(0.0, distance, 0.0),
            foot: glam::Vec3::new(0.0, distance, 0.0),
            radius,
            angular_radius: radius / distance,
            annulus: None,
            pole: glam::Vec3::Z,
            spread: Vec::new(),
        }
    }

    /// Nothing changes size at the crossover. Reading the one number as a radius in one place
    /// and a diameter in the other is a factor of two, with nothing in the types to catch it.
    #[test]
    fn a_body_holds_its_size_where_it_stops_being_a_sphere() {
        let view = viewport();
        let distance = 40.0;
        // A body sitting exactly on the threshold, and one a hair under it.
        let on = view.point_px * 0.5 * view.rad_per_px * distance;
        assert_eq!(form_of(&body_at(distance, on * 1.01), view), Form::Sphere, "just over");
        let under = body_at(distance, on * 0.99);
        assert_eq!(form_of(&under, view), Form::Circle, "just under");

        let drawn = item_transform(&under, view).scale.x;
        assert!(
            (drawn / on - 1.0).abs() < 0.05,
            "a circle of {drawn} where the sphere it replaced was {on}",
        );
    }

    /// A lighter thing gets a smaller mark, and never one too small to draw.
    #[test]
    fn a_lighter_mark_is_smaller_but_never_vanishes() {
        let view = viewport();
        let marked = |scale: f32| {
            let mut placement = body_at(40.0, 0.0);
            placement.symbol_scale = scale;
            view.mark_px(&placement)
        };
        assert!(marked(1.0) > marked(0.5), "half the scale should draw smaller");
        assert!(marked(0.5) >= POINT_FLOOR_PX, "and never under a shape's worth of pixels");
        assert_eq!(marked(1.0), view.point_px, "and a full weight is the surface's own size");
        for scale in [em_map::weight::MIN_SCALE, 0.0, 1.0e-9] {
            assert!(marked(scale) >= POINT_FLOOR_PX, "{scale} drew {}", marked(scale));
        }
        // The shader thickens a small mark's line to match, as a fraction of it; the shared cap
        // must leave room for the smallest there is, or it would be a hairline ring.
        for scale in [em_map::weight::MIN_SCALE, 0.0, 0.5, 1.0] {
            assert!(LINE_PX / (marked(scale) * 0.5) <= CIRCLE_TUBE_FRACTION, "{scale} was capped");
        }
    }

    /// The crossover is the surface's size for everyone, so a light body steps down to its
    /// mark. Stepping up would be a body growing as it recedes.
    #[test]
    fn a_light_body_steps_down_at_the_crossover_and_never_up() {
        let view = viewport();
        let distance = 40.0;
        let on = view.point_px * 0.5 * view.rad_per_px * distance;
        for scale in [1.0f32, 0.5, em_map::weight::MIN_SCALE] {
            let mut under = body_at(distance, on * 0.99);
            under.symbol_scale = scale;
            assert_eq!(form_of(&under, view), Form::Circle, "{scale} should be a mark");
            let drawn = item_transform(&under, view).scale.x;
            assert!(
                drawn <= on * 1.001,
                "a mark at scale {scale} drew {drawn}, bigger than the {on} sphere it replaced",
            );
        }
    }

    /// A mark is the same share of every surface. One texture serves a 190-point corner and a
    /// panel several times that.
    #[test]
    fn a_symbol_is_a_share_of_the_view_above_its_floor() {
        let fov = std::f32::consts::FRAC_PI_4;
        for height in [200u32, 410, 1080, 2160] {
            let share = Viewport::new(height, fov).point_px / height as f32;
            assert!(
                (share / POINT_FRACTION - 1.0).abs() < 1.0e-5,
                "a {height}-pixel surface drew a symbol at {share} of itself",
            );
        }
        // And below it the floor holds, which is where a ring has no inside left and the
        // honest answer is a dot.
        for height in [0u32, 1, 64, 159] {
            let px = Viewport::new(height, fov).point_px;
            assert!((px - POINT_FLOOR_PX).abs() < 1.0e-6, "{height} px drew a symbol of {px}");
        }
        assert!(POINT_FLOOR_PX <= 2.0 * LINE_PX, "the floor is twice a line and no more");
    }

    /// A disc comes out the size the transform says. Its vertices share one normal, so the
    /// displacement translates it; only asking for the base radius makes that zero.
    #[test]
    fn a_dot_is_never_displaced() {
        let view = viewport();
        for distance in [1.0e-3f32, 1.0, 40.0, 1.0e5] {
            let scale = point_radius(distance, view.rad_per_px, view.point_px);
            let target =
                tube_radius(scale, view.rad_per_px, distance, DOT_TUBE_FRACTION, LINE_PX);
            assert!(
                (target - BASE_TUBE_RADIUS).abs() < 1.0e-9,
                "at {distance:e} the disc would shift by {}",
                target - BASE_TUBE_RADIUS,
            );
        }
    }

    /// A ship is a mark at every zoom. Given a planet's radius it is still a mark.
    #[test]
    fn a_ship_never_becomes_a_model() {
        let view = viewport();
        // A radius that would fill the view, at a distance that would make anything else a
        // sphere many times over.
        let huge = kind_at(ItemKind::Ship, 1.0, 10.0);
        assert_eq!(form_of(&huge, view), Form::Dot, "a ship is never a sphere");
        assert_eq!(
            form_of(&kind_at(ItemKind::Planet, 1.0, 10.0), view),
            Form::Sphere,
            "and the exemption is the kind, not the numbers",
        );
        // Drawn at the symbol's own size, like every other mark.
        let at = item_transform(&huge, view);
        let px = 2.0 * at.scale.x / (at.translation.length() * view.rad_per_px);
        assert!((px / view.point_px - 1.0).abs() < 1.0e-3, "a ship drew {px} px across");
    }

    /// And it holds that size at every distance, so the far one reads as well as the near.
    #[test]
    fn a_circle_is_the_same_size_wherever_it_is() {
        let view = viewport();
        let want = view.point_px * 0.5;
        for distance in [1.0e-3f32, 1.0, 40.0, 1.0e5] {
            // Radius zero: nothing to draw at its own size, at any zoom.
            let at = item_transform(&body_at(distance, 0.0), view);
            let px = at.scale.x / (at.translation.length() * view.rad_per_px);
            assert!(
                (px / want - 1.0).abs() < 1.0e-3,
                "at {distance:e} the circle came out {px} px across the radius, wanted {want}",
            );
        }
    }

    /// A mark that does not face the eye is an ellipse, and edge-on a line. The eye is the
    /// render origin, because every transform on this layer is camera-relative.
    #[test]
    fn a_circle_faces_the_eye() {
        let view = viewport();
        let places = [
            glam::Vec3::new(0.0, 40.0, 0.0),
            glam::Vec3::new(-3.0, 0.5, 12.0),
            glam::Vec3::new(1.0e4, -2.0e3, 7.0),
            glam::Vec3::Y,
            -glam::Vec3::Y,
        ];
        for place in places {
            let mut placement = body_at(1.0, 0.0);
            placement.at = place;
            placement.foot = place;
            let at = item_transform(&placement, view);
            // The mesh's own normal is +Y; after the rotation it must point back at the eye.
            // Against the transform's own translation, not the placement's: these are two
            // different frames and `item_transform` is where the swizzle happens.
            let normal = at.rotation * Vec3::Y;
            let toward_eye = -at.translation.normalize();
            assert!(
                normal.dot(toward_eye) > 0.999,
                "at {place:?} the circle's normal was {normal:?}, wanted {toward_eye:?}",
            );
        }
        // And nothing blows up for something sitting on the camera.
        assert!(face_camera(Vec3::ZERO).is_finite());
    }

    /// A dash is the same length wherever it is drawn: the count follows the drop, so twice
    /// the drop is twice the dashes and not dashes twice as long.
    #[test]
    fn a_longer_drop_gets_more_dashes_not_longer_ones() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        let distance = 40.0;
        let dash_of = |height: f32| {
            let n = dash_count(height, distance, rad_per_px);
            // The mesh lays `n` dashes and `n - 1` gaps of equal length over the drop.
            height / (2 * n - 1) as f32
        };
        let want = distance * rad_per_px * DASH_PX;
        // Up to the cap, which is about a viewport's worth of line.
        for height in [0.5f32, 2.0, 9.0, 20.0] {
            let dash = dash_of(height);
            assert!(
                (dash / want - 1.0).abs() < 0.5,
                "a drop of {height} drew dashes of {dash} against a wanted {want}",
            );
        }
        assert!(
            dash_count(20.0, distance, rad_per_px) > dash_count(2.0, distance, rad_per_px),
            "a longer drop should have gained dashes",
        );
        // Past it the dashes do stretch, and that is the cap rather than the rule: a drop that
        // long runs several viewports off the screen and is not being read as dashes anyway.
        assert_eq!(dash_count(150.0, distance, rad_per_px), MAX_DASHES);
    }

    /// And it stays inside the meshes that exist, whatever it is handed.
    #[test]
    fn a_dash_count_is_always_a_mesh_there_is() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        for height in [0.0f32, -1.0, 1.0e-9, 1.0e9, f32::INFINITY, f32::NAN] {
            for distance in [0.0f32, 1.0e-6, 40.0, 1.0e5] {
                let n = dash_count(height, distance, rad_per_px);
                assert!((1..=MAX_DASHES).contains(&n), "{height} at {distance} gave {n}");
            }
        }
    }

    /// A line holds its screen width however hard its mesh is scaled, which is why the
    /// thickness is worked out from the view rather than baked in.
    #[test]
    fn a_line_holds_its_width_on_screen_across_the_scales() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        let width_px = |scale: f32, distance: f32| {
            let world =
                scale * tube_radius(scale, rad_per_px, distance, LINE_TUBE_FRACTION, SCALE_PX);
            world / distance / rad_per_px
        };
        for scale in [1.0f32, 1.0e2, 1.0e4] {
            let px = width_px(scale, scale);
            assert!((px - SCALE_PX).abs() < 1.0e-3, "{scale:e} units drew {px} px");
        }
    }
}
