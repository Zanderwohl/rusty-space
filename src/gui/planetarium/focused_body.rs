use bevy::prelude::*;
use bevy::math::DVec3;

#[derive(Resource, Default)]
pub struct FocusedBodyState {
    pub current_body_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HoveredTrajectoryMarkerKind {
    Periapsis,
    Apoapsis,
    MouseHit,
}

/// Data for a trajectory line segment hit from mouse hover raycasting.
#[derive(Clone)]
pub struct TrajectoryHitData {
    /// Index of the segment start point in the trajectory cache.
    pub segment_start_idx: usize,
    /// Relative time at segment start (within orbital period).
    pub start_time: f64,
    /// Relative time at segment end (within orbital period).
    pub end_time: f64,
    /// Interpolation parameter along the segment (0.0 = start, 1.0 = end).
    pub t: f64,
    /// Hit point position in simulation space (local to primary).
    pub hit_position_local: DVec3,
    /// Hit point position in bevy space.
    pub hit_position_bevy: Vec3,
}

#[derive(Resource, Default)]
pub struct HoverState {
    pub hovered_body_id: Option<String>,
    pub hovered_marker_kind: Option<HoveredTrajectoryMarkerKind>,
    /// Trajectory line hit data (only present when MouseHit marker should be shown).
    pub hovered_trajectory_hit: Option<TrajectoryHitData>,
}

impl FocusedBodyState {
    pub fn is_focused(&self, body_id: &str) -> bool {
        self.current_body_id.as_deref() == Some(body_id)
    }
}

impl HoverState {
    pub fn is_body_hovered(&self, body_id: &str) -> bool {
        self.hovered_body_id.as_deref() == Some(body_id)
    }
}
