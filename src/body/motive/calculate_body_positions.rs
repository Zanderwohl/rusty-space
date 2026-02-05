//! Unified body position calculation system.
//!
//! This module calculates positions for all bodies using the unified Motive system.
//! It handles:
//! 1. Building a dependency graph based on parent-child relationships
//! 2. Calculating Fixed and Keplerian positions in correct order (parents first)
//! 3. Calculating Newtonian positions affected by gravity from Major bodies
//!
//! Physics calculations are decoupled from frame rate - multiple time steps can be
//! processed per frame, with a configurable time budget to prevent frame drops.
//!
//! ## Performance Optimizations
//! - Uses Entity (u64, Copy) instead of String IDs to avoid allocations
//! - Caches PhysicsGraph as a Resource, only rebuilds when motives change
//! - Reuses PositionCache across frames
//! - Uses enum iterator to avoid Box<dyn Iterator> heap allocation

use std::collections::{HashMap, HashSet, VecDeque};
use bevy::math::DVec3;
use bevy::prelude::*;

use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::{Motive, MotiveSelection};
use crate::body::universe::Major;
use crate::body::universe::save::UniversePhysics;
use crate::gui::planetarium::time::{PreviousTimesIter, SimTime};
use crate::foundations::gravity;
use crate::foundations::time::Instant;
// ============================================================================
// Time Iterator (avoids Box<dyn Iterator> allocation)
// ============================================================================

/// Iterator over simulation times - either from queue or a single current time
enum TimeIter {
    Queued(PreviousTimesIter),
    Single(Option<f64>),
}

impl Iterator for TimeIter {
    type Item = f64;
    
    fn next(&mut self) -> Option<f64> {
        match self {
            TimeIter::Queued(iter) => iter.next(),
            TimeIter::Single(v) => v.take(),
        }
    }
}

// ============================================================================
// Data Structures for Dependency Graph (uses Entity instead of String)
// ============================================================================

/// Cached motive data for a body at the current time
#[derive(Clone)]
pub struct CachedMotive {
    /// Parent entity (None = relative to origin, only for hierarchical bodies)
    pub parent_entity: Option<Entity>,
    /// The motive selection at the current time
    pub selection: CachedMotiveSelection,
}

/// Simplified motive selection that only stores what's needed for position calculation
#[derive(Clone)]
pub enum CachedMotiveSelection {
    Fixed {
        position: DVec3,
    },
    Keplerian {
        /// Cloned KeplerMotive for displacement calculation
        kepler: crate::body::motive::kepler_motive::KeplerMotive,
        /// Parent mass (needed for mu calculation)
        parent_mass: f64,
    },
    Newtonian {
        position: DVec3,
        velocity: DVec3,
        /// The previous motive if this is a Release transition
        release_from_fixed: Option<(Option<Entity>, DVec3)>, // (parent_entity, fixed_position)
    },
}

/// Body metadata for physics calculations - uses Entity (Copy) instead of String
#[derive(Clone, Copy)]
pub struct BodyData {
    pub entity: Entity,
    pub mass: f64,
    pub is_major: bool,
}

/// Dependency graph for hierarchical body positioning.
/// Uses Entity as keys instead of String to avoid allocations.
#[derive(Resource, Default)]
pub struct PhysicsGraph {
    /// Map from Entity to its physics data
    pub body_data: HashMap<Entity, BodyData>,
    /// Cached motive data for each entity (avoids repeated motive_at() calls)
    pub cached_motives: HashMap<Entity, CachedMotive>,
    /// Topologically sorted Entities (parents before children) - only hierarchical bodies
    pub sorted_entities: Vec<Entity>,
    /// List of Newtonian body entities
    pub newtonian_entities: Vec<Entity>,
    /// Map from String ID to Entity (for primary_id lookups)
    pub id_to_entity: HashMap<String, Entity>,
    /// The last simulation time the graph was built for
    pub last_build_time: Instant,
    /// Whether the graph needs a full rebuild
    pub needs_rebuild: bool,
    /// Cached body count for pre-allocation
    last_body_count: usize,
    /// Cached major body count for pre-allocation
    last_major_count: usize,
}

impl PhysicsGraph {
    /// Clear all data but keep capacity for reuse
    pub fn clear(&mut self) {
        self.body_data.clear();
        self.cached_motives.clear();
        self.sorted_entities.clear();
        self.newtonian_entities.clear();
        self.id_to_entity.clear();
    }
    
    /// Reserve capacity based on previous body counts (avoids reallocation)
    pub fn reserve(&mut self, body_count: usize) {
        if self.body_data.capacity() < body_count {
            self.body_data.reserve(body_count - self.body_data.len());
            self.cached_motives.reserve(body_count - self.cached_motives.len());
            self.id_to_entity.reserve(body_count - self.id_to_entity.len());
            self.sorted_entities.reserve(body_count);
            self.newtonian_entities.reserve(body_count / 4); // Newtonian bodies are typically fewer
        }
    }
    
    /// Check if any motive has an event between last_time and current_time.
    /// Uses binary search for O(log n) per body instead of O(n).
    pub fn check_for_motive_changes(
        &self,
        bodies: &Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
        last_time: Instant,
        current_time: Instant,
    ) -> bool {
        for (_, _, motive, _, _) in bodies.iter() {
            // Binary search: O(log n) instead of iterating all events
            if motive.has_event_in_range(last_time, current_time) {
                return true;
            }
        }
        false
    }
}

/// Cached positions during a physics step.
/// Reused across frames to avoid allocations.
#[derive(Resource, Default)]
pub struct PositionCache {
    /// Calculated global positions keyed by Entity
    pub positions: HashMap<Entity, DVec3>,
    /// Major body data for Newtonian gravity calculations: (entity, mass, position)
    pub major_bodies: Vec<(Entity, f64, DVec3)>,
    /// Cached counts for pre-allocation
    last_body_count: usize,
    last_major_count: usize,
}

impl PositionCache {
    /// Clear for next step but keep capacity
    pub fn clear(&mut self) {
        self.positions.clear();
        // Don't clear major_bodies here - it's rebuilt separately and clearing twice is wasteful
    }
    
    /// Reserve capacity based on expected counts
    pub fn reserve(&mut self, body_count: usize, major_count: usize) {
        if self.positions.capacity() < body_count {
            self.positions.reserve(body_count - self.positions.len());
        }
        if self.major_bodies.capacity() < major_count {
            self.major_bodies.reserve(major_count - self.major_bodies.len());
        }
        self.last_body_count = body_count;
        self.last_major_count = major_count;
    }
    
    /// Clear major bodies for rebuild
    pub fn clear_major_bodies(&mut self) {
        self.major_bodies.clear();
    }
}

// ============================================================================
// Main System
// ============================================================================

/// System to calculate body positions based on the unified Motive component.
/// 
/// This runs multiple physics steps per frame based on `previous_times` in SimTime,
/// with a configurable time budget to prevent frame drops.
pub fn calculate_body_positions(
    mut sim_time: ResMut<SimTime>,
    physics: Res<UniversePhysics>,
    mut graph: ResMut<PhysicsGraph>,
    mut cache: ResMut<PositionCache>,
    mut bodies: Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
) {
    // Start frame timing
    sim_time.begin_frame();
    
    let current_time = sim_time.time;
    
    // Check if we need to rebuild the graph
    // Rebuild if: first time, bodies changed, or motive events occurred
    let needs_rebuild = graph.needs_rebuild 
        || graph.body_data.is_empty()
        || graph.check_for_motive_changes(&bodies, graph.last_build_time, current_time);
    
    if needs_rebuild {
        rebuild_physics_graph(&mut graph, &bodies, current_time);
        graph.needs_rebuild = false;
        graph.last_build_time = current_time;
        
        // Update cache capacity based on new counts from graph rebuild
        cache.reserve(graph.last_body_count, graph.last_major_count);
    }
    
    // Track how many steps we process and the last processed time
    let mut steps_processed = 0usize;
    let mut last_processed_time = current_time;
    
    // Determine which times to step through
    let has_queued_times = !sim_time.previous_times.is_empty();
    let total_steps = sim_time.previous_times.len();
    
    // Create an iterator over times (no heap allocation)
    let times_iter = if has_queued_times {
        TimeIter::Queued(sim_time.previous_times.iter())
    } else {
        TimeIter::Single(Some(current_time.to_j2000_seconds()))
    };
    
    // Process each time step
    for step_time in times_iter {
        let step_time = Instant::from_seconds_since_j2000(step_time);
        // Check if we've exceeded our frame time budget
        if sim_time.frame_time_exceeded() {
            break;
        }
        
        // Clear position cache for this step (keeps capacity)
        cache.clear();
        // Clear major bodies separately (only once, not in both clear() and update_major_body_cache())
        cache.clear_major_bodies();
        
        // Phase 1: Calculate Fixed and Keplerian positions
        calculate_hierarchical_positions(
            &mut bodies,
            &graph,
            &mut cache,
            step_time,
            physics.gravitational_constant,
        );
        
        // Update major body positions in cache for Newtonian calculations
        update_major_body_cache(&bodies, &graph, &mut cache);
        
        // Phase 2: Calculate Newtonian positions
        calculate_newtonian_positions(
            &mut bodies,
            &graph,
            &cache,
            step_time,
            sim_time.step,
            sim_time.playing,
            physics.gravitational_constant,
        );
        
        last_processed_time = step_time;
        sim_time.step_completed();
        steps_processed += 1;
    }
    
    // Update time_seconds to the last time we ACTUALLY processed
    sim_time.time = last_processed_time;
    
    // Remove only the times we actually processed (keep remaining for next frame)
    if has_queued_times {
        if steps_processed >= total_steps {
            sim_time.previous_times.clear();
        } else {
            sim_time.previous_times.drain_front(steps_processed);
        }
    }
    
    // End frame and calculate performance metrics
    sim_time.end_frame();
}

// ============================================================================
// Physics Graph Building
// ============================================================================

/// Rebuild the physics graph from scratch
fn rebuild_physics_graph(
    graph: &mut PhysicsGraph,
    bodies: &Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
    time: Instant,
) {
    // Count bodies for pre-allocation
    let body_count = bodies.iter().len();
    
    graph.clear();
    graph.reserve(body_count);
    graph.last_body_count = body_count;
    
    // First pass: build id_to_entity mapping and collect body data
    // Also count major bodies for later pre-allocation
    let mut major_count = 0usize;
    for (entity, info, _, _, major) in bodies.iter() {
        graph.id_to_entity.insert(info.id.clone(), entity);
        let is_major = major.is_some();
        if is_major {
            major_count += 1;
        }
        graph.body_data.insert(entity, BodyData {
            entity,
            mass: info.mass,
            is_major,
        });
    }
    graph.last_major_count = major_count;
    
    // Build temporary structures for topological sort
    // Using body_count as upper bound for hierarchical bodies
    let mut hierarchical_bodies: HashSet<Entity> = HashSet::with_capacity(body_count);
    let mut dependencies: HashMap<Entity, Option<Entity>> = HashMap::with_capacity(body_count);
    
    // Second pass: build cached motives and dependencies
    // This is the ONLY place we call motive_at() - results are cached
    for (entity, _info, motive, _, _) in bodies.iter() {
        let (event, selection) = motive.motive_at(time);
        
        match selection {
            MotiveSelection::Fixed { primary_id, position } => {
                let parent_entity = primary_id.as_ref()
                    .and_then(|id| graph.id_to_entity.get(id))
                    .copied();
                
                dependencies.insert(entity, parent_entity);
                hierarchical_bodies.insert(entity);
                
                graph.cached_motives.insert(entity, CachedMotive {
                    parent_entity,
                    selection: CachedMotiveSelection::Fixed { position: *position },
                });
            }
            MotiveSelection::Keplerian(kepler) => {
                let parent_entity = graph.id_to_entity.get(&kepler.primary_id).copied();
                let parent_mass = parent_entity
                    .and_then(|pe| graph.body_data.get(&pe))
                    .map(|d| d.mass)
                    .unwrap_or(0.0);
                
                dependencies.insert(entity, parent_entity);
                hierarchical_bodies.insert(entity);
                
                graph.cached_motives.insert(entity, CachedMotive {
                    parent_entity,
                    selection: CachedMotiveSelection::Keplerian {
                        kepler: kepler.clone(),
                        parent_mass,
                    },
                });
            }
            MotiveSelection::Newtonian { position, velocity } => {
                // Check if this is a Release transition and cache the previous Fixed motive data
                let release_from_fixed = if matches!(event, crate::body::motive::TransitionEvent::Release) {
                    if let Some((_, MotiveSelection::Fixed { primary_id, position: fixed_pos })) = motive.motive_before(time) {
                        let prev_parent_entity = primary_id.as_ref()
                            .and_then(|id| graph.id_to_entity.get(id))
                            .copied();
                        Some((prev_parent_entity, *fixed_pos))
                    } else {
                        None
                    }
                } else {
                    None
                };
                
                graph.newtonian_entities.push(entity);
                graph.cached_motives.insert(entity, CachedMotive {
                    parent_entity: None, // Newtonian bodies don't have hierarchical parents
                    selection: CachedMotiveSelection::Newtonian {
                        position: *position,
                        velocity: *velocity,
                        release_from_fixed,
                    },
                });
            }
        }
    }
    
    // Topologically sort hierarchical bodies
    graph.sorted_entities = topological_sort_optimized(&hierarchical_bodies, &dependencies);
}

// ============================================================================
// Hierarchical Position Calculation (Fixed & Keplerian)
// ============================================================================

/// Calculate positions for Fixed and Keplerian bodies in dependency order.
/// Uses cached parent/mass data but calls motive_at() fresh for each body.
fn calculate_hierarchical_positions(
    bodies: &mut Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
    graph: &PhysicsGraph,
    cache: &mut PositionCache,
    time: Instant,
    gravitational_constant: f64,
) {
    // Calculate positions in topological order
    for &entity in &graph.sorted_entities {
        // Get cached motive data for parent info
        let Some(cached_motive) = graph.cached_motives.get(&entity) else { continue };
        
        // Get the body from the query - we need the motive to calculate position
        let Ok((_, _, motive, mut state, _)) = bodies.get_mut(entity) else { continue };
        
        // Get parent position from cache (parent is guaranteed to be processed first due to topo sort)
        let parent_position = cached_motive.parent_entity
            .and_then(|pe| cache.positions.get(&pe))
            .copied()
            .unwrap_or(DVec3::ZERO);
        
        // Get fresh motive selection at current time
        let (_, selection) = motive.motive_at(time);
        
        // Calculate local position based on motive selection
        let local_position = match selection {
            MotiveSelection::Fixed { position, .. } => {
                *position
            }
            MotiveSelection::Keplerian(kepler) => {
                // Get parent mass from cached data
                let parent_mass = match &cached_motive.selection {
                    CachedMotiveSelection::Keplerian { parent_mass, .. } => *parent_mass,
                    _ => 0.0, // Shouldn't happen
                };
                let mu = gravitational_constant * parent_mass;
                kepler.displacement(time, mu).unwrap_or(DVec3::ZERO)
            }
            MotiveSelection::Newtonian { .. } => {
                // Skip - handled in phase 2 (shouldn't be in sorted_entities anyway)
                continue;
            }
        };
        
        let global_position = parent_position + local_position;
        
        // Update body state
        state.current_position = global_position;
        state.current_local_position = Some(local_position);
        state.current_primary_position = if cached_motive.parent_entity.is_some() { 
            Some(parent_position) 
        } else { 
            None 
        };
        
        // Cache position for children
        cache.positions.insert(entity, global_position);
    }
}

/// Update the major body cache with current positions for Newtonian calculations.
/// Note: cache.clear_major_bodies() should be called before this to avoid duplicates.
fn update_major_body_cache(
    bodies: &Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
    graph: &PhysicsGraph,
    cache: &mut PositionCache,
) {
    // Clear is handled by caller - don't double-clear
    for (&entity, &body_data) in &graph.body_data {
        if body_data.is_major {
            if let Ok((_, _, _, state, _)) = bodies.get(entity) {
                cache.major_bodies.push((entity, body_data.mass, state.current_position));
            }
        }
    }
}

// ============================================================================
// Newtonian Position Calculation
// ============================================================================

/// Calculate positions for Newtonian bodies using gravity from Major bodies.
/// 
/// This function handles:
/// - Standard Newtonian integration using velocity stored in BodyState
/// - Initialization of Newtonian state when first entering a Newtonian motive
/// - Release transitions from Fixed to Newtonian (computing position and transforming velocity)
/// 
/// Uses cached motive data to avoid repeated motive_at() calls.
fn calculate_newtonian_positions(
    bodies: &mut Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
    graph: &PhysicsGraph,
    cache: &PositionCache,
    time: Instant,
    delta_time: f64,
    playing: bool,
    gravitational_constant: f64,
) {
    let effective_delta = if playing { delta_time } else { 0.0 };
    
    // Process each Newtonian body
    for &entity in &graph.newtonian_entities {
        // Get cached motive data
        let Some(cached_motive) = graph.cached_motives.get(&entity) else { continue };
        
        let CachedMotiveSelection::Newtonian { position, velocity, release_from_fixed } = &cached_motive.selection else {
            continue; // Shouldn't happen - newtonian_entities should only contain Newtonian bodies
        };
        
        if let Ok((_, _, _, mut state, _)) = bodies.get_mut(entity) {
            // Check if we need to initialize/reinitialize the Newtonian state
            let is_release = release_from_fixed.is_some();
            let needs_init = state.current_velocity.is_none() 
                || state.newtonian_init_time.is_none()
                || is_release;
            
            let (mut current_pos, mut current_vel) = if needs_init {
                // Handle Release transition: compute position from cached previous Fixed motive
                if let Some((prev_parent_entity, fixed_pos)) = release_from_fixed {
                    // Compute the global position from the cached Fixed motive data
                    let parent_pos = prev_parent_entity
                        .and_then(|pe| cache.positions.get(&pe))
                        .copied()
                        .unwrap_or(DVec3::ZERO);
                    
                    let global_pos = parent_pos + *fixed_pos;
                    
                    // The velocity in the Newtonian motive is the LOCAL velocity
                    // For now, use it as-is (assumes the local frame is aligned with global)
                    // TODO: Could add velocity transformation if parent has velocity
                    let global_vel = *velocity;
                    
                    state.newtonian_init_time = Some(time);
                    (global_pos, global_vel)
                } else {
                    // Standard initialization: use stored position/velocity
                    state.newtonian_init_time = Some(time);
                    (*position, *velocity)
                }
            } else {
                // Use the current state from previous integration step
                (state.current_position, state.current_velocity.unwrap_or(*velocity))
            };
            
            if effective_delta.abs() > f64::EPSILON {
                // Calculate gravitational acceleration from all Major bodies
                let acceleration: DVec3 = cache.major_bodies.iter()
                    .filter(|(e, _, _)| *e != entity) // Don't apply self-gravity
                    .map(|(_, mass, pos)| {
                        let a_to_b = current_pos - *pos;
                        gravity::one_body_acceleration(gravitational_constant * mass, a_to_b)
                    })
                    .sum();
                
                // Update position and velocity using simple Euler integration
                // TODO: Consider using Verlet or RK4 for better accuracy
                current_pos += current_vel * effective_delta;
                current_vel += acceleration * effective_delta;
            }
            
            state.current_position = current_pos;
            state.current_velocity = Some(current_vel);
            state.last_step_position = *position;
            state.current_local_position = None;
            state.current_primary_position = None;
        }
    }
}

// ============================================================================
// Topological Sort (uses Entity instead of String)
// ============================================================================

/// Optimized topological sort of entities based on parent-child dependencies.
/// Returns entities sorted so that parents come before children.
/// 
/// Optimizations:
/// - Pre-allocates all collections with known capacity
/// - Builds children map and roots in a single pass
/// - Only does fallback iteration if BFS didn't process all bodies (rare case)
fn topological_sort_optimized(
    bodies: &HashSet<Entity>,
    dependencies: &HashMap<Entity, Option<Entity>>,
) -> Vec<Entity> {
    let body_count = bodies.len();
    let mut result = Vec::with_capacity(body_count);
    let mut visited: HashSet<Entity> = HashSet::with_capacity(body_count);
    
    // Build reverse dependency map (parent -> children) and find roots in one pass
    // Estimate: average ~3 children per parent, but cap at body_count
    let mut children: HashMap<Entity, Vec<Entity>> = HashMap::with_capacity(body_count / 2);
    let mut roots: Vec<Entity> = Vec::with_capacity(body_count / 4); // Roots are typically fewer
    
    for &entity in bodies {
        if let Some(Some(parent)) = dependencies.get(&entity) {
            children.entry(*parent).or_insert_with(|| Vec::with_capacity(4)).push(entity);
        } else {
            roots.push(entity);
        }
    }
    
    // BFS from roots to ensure proper ordering
    let mut queue: VecDeque<Entity> = VecDeque::with_capacity(body_count);
    queue.extend(roots);
    
    while let Some(entity) = queue.pop_front() {
        if visited.contains(&entity) {
            continue;
        }
        
        // Check if parent has been visited (if there is a parent)
        if let Some(Some(parent)) = dependencies.get(&entity) {
            if !visited.contains(parent) && bodies.contains(parent) {
                queue.push_back(entity);
                continue;
            }
        }
        
        visited.insert(entity);
        result.push(entity);
        
        // Add children to queue
        if let Some(child_entities) = children.get(&entity) {
            for &child in child_entities {
                if !visited.contains(&child) {
                    queue.push_back(child);
                }
            }
        }
    }
    
    // Handle any remaining bodies (circular dependencies or orphans)
    // Only iterate if we haven't processed all bodies yet
    if result.len() < body_count {
        for &entity in bodies {
            if !visited.contains(&entity) {
                result.push(entity);
            }
        }
    }
    
    result
}
