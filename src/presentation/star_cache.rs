//! Per-frame star data cache for lighting calculations.
//!
//! Builds a shared snapshot of star positions and intensities once per frame,
//! avoiding repeated query iteration and vector allocation in multiple systems.

use bevy::math::DVec3;
use bevy::prelude::*;

use crate::body::appearance::Appearance;
use crate::body::motive::info::BodyState;

/// Cached star data for a single star entity.
#[derive(Clone)]
pub struct CachedStarData {
    pub entity: Entity,
    /// Position in Bevy world space.
    pub bevy_position: Vec3,
    /// Position in simulation space (from BodyState.current_position)
    pub sim_position: DVec3,
    /// Star intensity for brightness calculations
    pub intensity: f32,
}

/// Per-frame cache of star data for lighting systems.
/// 
/// Built once per frame by `build_star_lighting_cache`, then consumed by:
/// - `update_body_points`
/// - `update_wireframe_lighting`
/// - `update_occluder_lighting`
#[derive(Resource, Default)]
pub struct StarLightingFrameCache {
    /// All star entities with their cached data
    pub stars: Vec<CachedStarData>,
    /// Maximum intensity across all stars (for normalization)
    pub max_intensity: f32,
}

impl StarLightingFrameCache {
    /// Clear the cache, keeping capacity for reuse.
    pub fn clear(&mut self) {
        self.stars.clear();
        self.max_intensity = 0.0;
    }
}

/// System to build the star lighting cache once per frame.
/// Must run before all systems that consume star data.
pub fn build_star_lighting_cache(
    mut cache: ResMut<StarLightingFrameCache>,
    stars: Query<(Entity, &GlobalTransform, &Appearance, &BodyState)>,
) {
    cache.clear();
    
    for (entity, global_transform, appearance, body_state) in stars.iter() {
        if let Appearance::Star(star_ball) = appearance {
            let intensity = star_ball.intensity();
            cache.stars.push(CachedStarData {
                entity,
                bevy_position: global_transform.translation(),
                sim_position: body_state.current_position,
                intensity,
            });
            
            if intensity > cache.max_intensity {
                cache.max_intensity = intensity;
            }
        }
    }
}
