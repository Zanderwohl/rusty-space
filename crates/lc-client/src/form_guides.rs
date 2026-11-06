//! The midlines Shift holds a carried part to, drawn on its parent as a loop round each center
//! plane, on the handles' layer so the hull never buries them.

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use glam::DVec3;
use lc_world::form::primitive::Shape;

use crate::app::Ui;
use crate::form_carry::{Carried, Guide};
use crate::form_handles::{FORM_HANDLE_LAYER, lens, original};
use crate::form_view::{FormSurface, Shown};

const SEGMENTS: usize = 96;
const LINE_PX: f64 = 2.0;
const COLOR: Color = Color::srgb(0.85, 0.85, 0.80);

pub struct FormGuidesPlugin;

impl Plugin for FormGuidesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, make_looks).add_systems(
            Update,
            draw.in_set(crate::app::Stage::Scene).after(crate::form_view::place).run_if(in_state(crate::app::AppState::InGame)),
        );
    }
}

#[derive(Resource)]
struct Looks {
    line: Handle<Mesh>,
    paint: Handle<StandardMaterial>,
}

fn make_looks(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(Looks {
        line: meshes.add(Cylinder::new(0.5, 1.0)),
        paint: materials.add(StandardMaterial { base_color: COLOR, unlit: true, ..default() }),
    });
}

#[derive(Component)]
struct GuideRoot(Guide);

#[derive(Component)]
struct Segment(usize);

/// Where each ray from the center in the plane `axis` = 0 leaves, so the loop lies on the surface
/// an anchor is solved on.
pub fn midline(shape: &Shape, axis: usize) -> Vec<DVec3> {
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    (0..=SEGMENTS)
        .map(|k| {
            let (s, c) = (k as f64 / SEGMENTS as f64 * std::f64::consts::TAU).sin_cos();
            let mut d = DVec3::ZERO;
            d[a] = c;
            d[b] = s;
            shape.exit(d).point
        })
        .collect()
}

fn draw(
    mut commands: Commands,
    ui: Res<Ui>,
    shown: Res<Shown>,
    surface: Res<FormSurface>,
    carried: Res<Carried>,
    looks: Res<Looks>,
    roots: Query<(Entity, &GuideRoot)>,
    mut segments: Query<(&Segment, &mut Transform)>,
) {
    let guide = carried.carry().and_then(|c| c.guide).filter(|_| ui.view == crate::ui::ViewMode::Form);
    let found = guide.zip(lens(&ui, &shown, &surface)).zip(shown.sdf()).and_then(|((guide, lens), sdf)| {
        let (_, host) = original(sdf, guide.host)?;
        let loops: Vec<Vec<DVec3>> =
            guide.axes.iter().map(|&axis| midline(&host.shape, axis).into_iter().map(|p| host.pose.to_outer(p)).collect()).collect();
        Some((guide, lens, loops))
    });
    let current = roots.iter().next().map(|(_, root)| root.0);
    if current != found.as_ref().map(|(guide, ..)| *guide) {
        for (entity, _) in &roots {
            commands.entity(entity).despawn();
        }
        let Some((guide, ..)) = &found else { return };
        let turn = crate::hull::frame(DVec3::X, Some(DVec3::Z), 0.0);
        let root = commands.spawn((Transform::from_rotation(turn), Visibility::default(), GuideRoot(*guide))).id();
        for i in 0..guide.axes.len() * SEGMENTS {
            commands.spawn((
                Mesh3d(looks.line.clone()),
                MeshMaterial3d(looks.paint.clone()),
                Transform::default(),
                NoFrustumCulling,
                RenderLayers::layer(FORM_HANDLE_LAYER),
                Segment(i),
                ChildOf(root),
            ));
        }
        return;
    }
    let Some((_, lens, loops)) = found else { return };
    for (segment, mut transform) in &mut segments {
        let (ring, k) = (segment.0 / SEGMENTS, segment.0 % SEGMENTS);
        let (from, to) = (loops[ring][k], loops[ring][k + 1]);
        let depth = lens.project((from + to) * 0.5).map_or(1.0, |(_, depth)| depth);
        transform.set_if_neq(crate::form_handles::laid(from, to, LINE_PX * lens.m_per_px(depth)));
    }
}

#[cfg(test)]
mod tests {
    use lc_world::form::primitive::Shape;

    use super::*;

    #[test]
    fn a_midline_is_a_closed_loop_on_the_surface_in_its_plane() {
        let slab = Shape::Slab { edges: DVec3::new(360.0, 160.0, 40.0), corner: 4.0 };
        for axis in 0..3 {
            let line = midline(&slab, axis);
            assert_eq!(line.first().map(|p| (*p - line[SEGMENTS]).length() < 1e-9), Some(true));
            for p in &line {
                assert_eq!(p[axis], 0.0);
                let d = p.normalize();
                assert!((slab.exit(d).point - *p).length() < 1e-9);
            }
        }
    }
}
