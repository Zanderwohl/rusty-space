//! Unified body position calculation system.
//!
//! This module calculates positions for all bodies using the unified Motive system.
//! It handles:
//! 1. Building a dependency graph based on parent-child relationships
//! 2. Calculating Fixed and Keplerian positions in correct order (parents first)
//! 3. Calculating Newtonian positions affected by gravity from Major bodies

use std::collections::{HashMap, HashSet, VecDeque};
use bevy::math::DVec3;
use bevy::prelude::*;

use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::{Motive, MotiveSelection};
use crate::body::universe::Major;
use crate::body::universe::save::UniversePhysics;
use crate::gui::planetarium::time::SimTime;
use crate::util::gravity;

/// System to calculate body positions based on the unified Motive component.
/// 
/// This runs in two phases:
/// 1. Fixed and Keplerian bodies (in dependency order - parents before children)
/// 2. Newtonian bodies (affected by gravity from Major bodies)
pub fn calculate_body_positions(
    sim_time: Res<SimTime>,
    physics: Res<UniversePhysics>,
    mut bodies: Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
) {
    let time = sim_time.time_seconds;
    
    // Phase 1: Build dependency graph and calculate Fixed/Keplerian positions
    calculate_hierarchical_positions(&mut bodies, time, physics.gravitational_constant);
    
    // Phase 2: Calculate Newtonian positions
    calculate_newtonian_positions(&mut bodies, &sim_time, physics.gravitational_constant);
}

/// Calculate positions for Fixed and Keplerian bodies in dependency order.
fn calculate_hierarchical_positions(
    bodies: &mut Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
    time: f64,
    gravitational_constant: f64,
) {
    // Collect body data for dependency resolution
    let mut body_data: HashMap<String, (Entity, f64)> = HashMap::new(); // id -> (entity, mass)
    let mut dependencies: HashMap<String, Option<String>> = HashMap::new(); // id -> parent_id
    let mut hierarchical_bodies: HashSet<String> = HashSet::new();
    
    for (entity, info, motive, _, _) in bodies.iter() {
        body_data.insert(info.id.clone(), (entity, info.mass));
        
        let (_, selection) = motive.motive_at(time);
        match selection {
            MotiveSelection::Fixed { primary_id, .. } => {
                dependencies.insert(info.id.clone(), primary_id.clone());
                hierarchical_bodies.insert(info.id.clone());
            }
            MotiveSelection::Keplerian(kepler) => {
                dependencies.insert(info.id.clone(), Some(kepler.primary_id.clone()));
                hierarchical_bodies.insert(info.id.clone());
            }
            MotiveSelection::Newtonian { .. } => {
                // Newtonian bodies are handled in phase 2
            }
        }
    }
    
    // Topological sort: find bodies with no dependencies first, then their children
    let sorted_ids = topological_sort(&hierarchical_bodies, &dependencies);
    
    // Store calculated positions for parent lookups
    let mut calculated_positions: HashMap<String, DVec3> = HashMap::new();
    
    // Calculate positions in order
    for body_id in sorted_ids {
        let Some(&(entity, mass)) = body_data.get(&body_id) else { continue };
        
        // Get the parent position (or origin if no parent)
        let parent_id = dependencies.get(&body_id).and_then(|p| p.clone());
        let parent_position = parent_id
            .as_ref()
            .and_then(|pid| calculated_positions.get(pid))
            .copied()
            .unwrap_or(DVec3::ZERO);
        
        // Get parent mass for Keplerian calculations
        let parent_mass = parent_id
            .as_ref()
            .and_then(|pid| body_data.get(pid))
            .map(|(_, m)| *m)
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
            state.current_primary_position = if parent_id.is_some() { Some(parent_position) } else { None };
            
            calculated_positions.insert(body_id, global_position);
        }
    }
}

/// Calculate positions for Newtonian bodies using gravity from Major bodies.
fn calculate_newtonian_positions(
    bodies: &mut Query<(Entity, &BodyInfo, &Motive, &mut BodyState, Option<&Major>)>,
    sim_time: &SimTime,
    gravitational_constant: f64,
) {
    // Collect Major body positions and masses
    let mut major_bodies: Vec<(String, f64, DVec3)> = Vec::new(); // (id, mass, position)
    let mut newtonian_entities: Vec<Entity> = Vec::new();
    
    for (entity, info, motive, state, major) in bodies.iter() {
        // Collect all Major bodies (regardless of motive type)
        if major.is_some() {
            major_bodies.push((info.id.clone(), info.mass, state.current_position));
        }
        
        // Identify Newtonian bodies
        let (_, selection) = motive.motive_at(sim_time.time_seconds);
        if matches!(selection, MotiveSelection::Newtonian { .. }) {
            newtonian_entities.push(entity);
        }
    }
    
    // Calculate time step
    let delta_time = if sim_time.playing {
        sim_time.step
    } else {
        0.0
    };
    
    // Process each Newtonian body
    for entity in newtonian_entities {
        if let Ok((_, info, motive, mut state, _)) = bodies.get_mut(entity) {
            let (_, selection) = motive.motive_at(sim_time.time_seconds);
            
            if let MotiveSelection::Newtonian { position, velocity } = selection {
                let mut current_pos = *position;
                let mut current_vel = *velocity;
                
                if delta_time.abs() > f64::EPSILON {
                    // Calculate gravitational acceleration from all Major bodies
                    let acceleration: DVec3 = major_bodies.iter()
                        .filter(|(id, _, _)| id != &info.id) // Don't apply self-gravity
                        .map(|(_, mass, pos)| {
                            let a_to_b = current_pos - *pos;
                            gravity::one_body_acceleration(gravitational_constant * mass, a_to_b)
                        })
                        .sum();
                    
                    // Update position and velocity using simple Euler integration
                    // TODO: Consider using Verlet or RK4 for better accuracy
                    current_pos += current_vel * delta_time;
                    current_vel += acceleration * delta_time;
                }
                
                state.current_position = current_pos;
                state.last_step_position = *position;
                state.current_local_position = None;
                state.current_primary_position = None;
                
                // Note: The actual motive position/velocity should be updated elsewhere
                // if we want persistence between frames. This system only updates BodyState.
            }
        }
    }
}

/// Topological sort of body IDs based on parent-child dependencies.
/// Returns bodies sorted so that parents come before children.
fn topological_sort(
    bodies: &HashSet<String>,
    dependencies: &HashMap<String, Option<String>>,
) -> Vec<String> {
    let mut result = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    
    // Build reverse dependency map (parent -> children)
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    let mut roots: Vec<String> = Vec::new();
    
    for body_id in bodies {
        if let Some(parent_id) = dependencies.get(body_id).and_then(|p| p.clone()) {
            children.entry(parent_id).or_default().push(body_id.clone());
        } else {
            roots.push(body_id.clone());
        }
    }
    
    // BFS from roots to ensure proper ordering
    let mut queue: VecDeque<String> = VecDeque::from(roots);
    
    while let Some(body_id) = queue.pop_front() {
        if visited.contains(&body_id) {
            continue;
        }
        
        // Check if parent has been visited (if there is a parent)
        if let Some(Some(parent_id)) = dependencies.get(&body_id) {
            if !visited.contains(parent_id) && bodies.contains(parent_id) {
                // Parent not yet visited, defer this body
                queue.push_back(body_id);
                continue;
            }
        }
        
        visited.insert(body_id.clone());
        result.push(body_id.clone());
        
        // Add children to queue
        if let Some(child_ids) = children.get(&body_id) {
            for child_id in child_ids {
                if !visited.contains(child_id) {
                    queue.push_back(child_id.clone());
                }
            }
        }
    }
    
    // Handle any remaining bodies (circular dependencies or orphans)
    for body_id in bodies {
        if !visited.contains(body_id) {
            result.push(body_id.clone());
        }
    }
    
    result
}

