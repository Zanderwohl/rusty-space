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
use crate::util::gravity;

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
    /// Map from Entity to parent Entity (None = relative to origin)
    pub dependencies: HashMap<Entity, Option<Entity>>,
    /// Set of Entities that use hierarchical positioning (Fixed or Keplerian)
    pub hierarchical_bodies: HashSet<Entity>,
    /// Topologically sorted Entities (parents before children)
    pub sorted_entities: Vec<Entity>,
    /// List of Newtonian body entities
    pub newtonian_entities: Vec<Entity>,
    /// Map from String ID to Entity (for primary_id lookups)
    pub id_to_entity: HashMap<String, Entity>,
    /// The last simulation time the graph was built for
    pub last_build_time: f64,
    /// Whether the graph needs a full rebuild
    pub needs_rebuild: bool,
}

impl PhysicsGraph {
    /// Clear all data but keep capacity
    pub fn clear(&mut self) {
        self.body_data.clear();
        self.dependencies.clear();
        self.hierarchical_bodies.clear();
        self.sorted_entities.clear();
        self.newtonian_entities.clear();
        self.id_to_entity.clear();
    }
    
    /// Check if any motive has an event between last_time and current_time.
    /// Uses binary search for O(log n) per body instead of O(n).
    pub fn check_for_motive_changes(
        &self,
        bodies: &Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
        last_time: f64,
        current_time: f64,
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
}

impl PositionCache {
    /// Clear for next step but keep capacity
    pub fn clear(&mut self) {
        self.positions.clear();
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
    
    let current_time = sim_time.time_seconds;
    
    // Check if we need to rebuild the graph
    // Rebuild if: first time, bodies changed, or motive events occurred
    let needs_rebuild = graph.needs_rebuild 
        || graph.body_data.is_empty()
        || graph.check_for_motive_changes(&bodies, graph.last_build_time, current_time);
    
    if needs_rebuild {
        rebuild_physics_graph(&mut graph, &bodies, current_time);
        graph.needs_rebuild = false;
        graph.last_build_time = current_time;
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
        TimeIter::Single(Some(current_time))
    };
    
    // Process each time step
    for step_time in times_iter {
        // Check if we've exceeded our frame time budget
        if sim_time.frame_time_exceeded() {
            break;
        }
        
        // Clear position cache for this step (keeps capacity)
        cache.clear();
        
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
    sim_time.time_seconds = last_processed_time;
    
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
    time: f64,
) {
    graph.clear();
    
    // First pass: build id_to_entity mapping and collect body data
    for (entity, info, _, _, major) in bodies.iter() {
        graph.id_to_entity.insert(info.id.clone(), entity);
        graph.body_data.insert(entity, BodyData {
            entity,
            mass: info.mass,
            is_major: major.is_some(),
        });
    }
    
    // Second pass: build dependencies using entity references
    for (entity, info, motive, _, _) in bodies.iter() {
        let (_, selection) = motive.motive_at(time);
        match selection {
            MotiveSelection::Fixed { primary_id, .. } => {
                let parent_entity = primary_id.as_ref()
                    .and_then(|id| graph.id_to_entity.get(id))
                    .copied();
                graph.dependencies.insert(entity, parent_entity);
                graph.hierarchical_bodies.insert(entity);
            }
            MotiveSelection::Keplerian(kepler) => {
                let parent_entity = graph.id_to_entity.get(&kepler.primary_id).copied();
                graph.dependencies.insert(entity, parent_entity);
                graph.hierarchical_bodies.insert(entity);
            }
            MotiveSelection::Newtonian { .. } => {
                graph.newtonian_entities.push(entity);
            }
        }
    }
    
    // Topologically sort hierarchical bodies
    graph.sorted_entities = topological_sort(&graph.hierarchical_bodies, &graph.dependencies);
}

// ============================================================================
// Hierarchical Position Calculation (Fixed & Keplerian)
// ============================================================================

/// Calculate positions for Fixed and Keplerian bodies in dependency order.
fn calculate_hierarchical_positions(
    bodies: &mut Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
    graph: &PhysicsGraph,
    cache: &mut PositionCache,
    time: f64,
    gravitational_constant: f64,
) {
    // Calculate positions in topological order
    for &entity in &graph.sorted_entities {
        let Some(&body_data) = graph.body_data.get(&entity) else { continue };
        
        // Get the parent entity and position (or origin if no parent)
        let parent_entity = graph.dependencies.get(&entity).and_then(|p| *p);
        let parent_position = parent_entity
            .and_then(|pe| cache.positions.get(&pe))
            .copied()
            .unwrap_or(DVec3::ZERO);
        
        // Get parent mass for Keplerian calculations
        let parent_mass = parent_entity
            .and_then(|pe| graph.body_data.get(&pe))
            .map(|d| d.mass)
            .unwrap_or(0.0);
        
        // Calculate this body's position
        if let Ok((_, _, motive, mut state, _)) = bodies.get_mut(entity) {
            let (_, selection) = motive.motive_at(time);
            
            let local_position = match selection {
                MotiveSelection::Fixed { position, .. } => {
                    *position
                }
                MotiveSelection::Keplerian(kepler) => {
                    let mu = gravitational_constant * parent_mass;
                    kepler.displacement(time, mu).unwrap_or(DVec3::ZERO)
                }
                MotiveSelection::Newtonian { .. } => {
                    // Skip - handled in phase 2
                    continue;
                }
            };
            
            let global_position = parent_position + local_position;
            
            state.current_position = global_position;
            state.current_local_position = Some(local_position);
            state.current_primary_position = if parent_entity.is_some() { Some(parent_position) } else { None };
            
            cache.positions.insert(entity, global_position);
        }
    }
}

/// Update the major body cache with current positions for Newtonian calculations
fn update_major_body_cache(
    bodies: &Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
    graph: &PhysicsGraph,
    cache: &mut PositionCache,
) {
    cache.major_bodies.clear();
    
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
fn calculate_newtonian_positions(
    bodies: &mut Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
    graph: &PhysicsGraph,
    cache: &PositionCache,
    time: f64,
    delta_time: f64,
    playing: bool,
    gravitational_constant: f64,
) {
    use crate::body::motive::TransitionEvent;
    
    let effective_delta = if playing { delta_time } else { 0.0 };
    
    // Process each Newtonian body
    for &entity in &graph.newtonian_entities {
        if let Ok((_, _, motive, mut state, _)) = bodies.get_mut(entity) {
            let (event, selection) = motive.motive_at(time);
            
            if let MotiveSelection::Newtonian { position, velocity } = selection {
                // Check if we need to initialize/reinitialize the Newtonian state
                let needs_init = state.current_velocity.is_none() 
                    || state.newtonian_init_time.is_none()
                    || matches!(event, TransitionEvent::Release);
                
                let (mut current_pos, mut current_vel) = if needs_init {
                    // Handle Release transition: compute position from previous Fixed motive
                    if matches!(event, TransitionEvent::Release) {
                        // For Release, the Newtonian velocity is LOCAL (relative to parent's frame)
                        // We need to find the parent and transform to global coordinates
                        
                        // Get the previous motive (should be Fixed)
                        let prev_motive = motive.motive_before(time);
                        
                        if let Some((_, MotiveSelection::Fixed { primary_id, position: fixed_pos })) = prev_motive {
                            // Compute the global position from the Fixed motive
                            let parent_pos = primary_id.as_ref()
                                .and_then(|id| graph.id_to_entity.get(id))
                                .and_then(|&pe| cache.positions.get(&pe))
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
                            // Fallback: use the stored position/velocity
                            state.newtonian_init_time = Some(time);
                            (*position, *velocity)
                        }
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
}

// ============================================================================
// Topological Sort (uses Entity instead of String)
// ============================================================================

/// Topological sort of entities based on parent-child dependencies.
/// Returns entities sorted so that parents come before children.
fn topological_sort(
    bodies: &HashSet<Entity>,
    dependencies: &HashMap<Entity, Option<Entity>>,
) -> Vec<Entity> {
    let mut result = Vec::with_capacity(bodies.len());
    let mut visited: HashSet<Entity> = HashSet::with_capacity(bodies.len());
    
    // Build reverse dependency map (parent -> children)
    let mut children: HashMap<Entity, Vec<Entity>> = HashMap::new();
    let mut roots: Vec<Entity> = Vec::new();
    
    for &entity in bodies {
        if let Some(Some(parent)) = dependencies.get(&entity) {
            children.entry(*parent).or_default().push(entity);
        } else {
            roots.push(entity);
        }
    }
    
    // BFS from roots to ensure proper ordering
    let mut queue: VecDeque<Entity> = VecDeque::from(roots);
    
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
    for &entity in bodies {
        if !visited.contains(&entity) {
            result.push(entity);
        }
    }
    
    result
}
