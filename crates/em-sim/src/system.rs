//! The simulation arena: every body, its motion, and the state propagation writes.
//!
//! `System` is the single source of truth for body state; nothing per-body lives elsewhere.
//! Engine-free — propagation is free functions in [`crate::propagate`]. Struct-of-arrays
//! keyed by [`BodyIndex`], with a `BodyId -> BodyIndex` map; derived columns (parent,
//! topological order, mu) are rebuilt when the structure changes.

use std::collections::HashMap;

use em_foundations::time::Instant;
use glam::DVec3;

use crate::appearance::Appearance;
use crate::body::{BodyInfo, BodyRotation};
use crate::id::{BodyId, BodyIndex};
use crate::motive::{Motive, MotiveSelection};

#[derive(Debug, Clone, PartialEq)]
pub enum SystemError {
    /// Two bodies share a name, and therefore an id.
    DuplicateName(String),
    /// Different names, same id. Refused rather than silently merging two bodies.
    HashCollision { existing: String, incoming: String },
    /// A body orbits a primary that is not in the file.
    UnresolvedPrimary { body: String, primary: String },
}

impl std::fmt::Display for SystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemError::DuplicateName(n) => write!(f, "a body named {n:?} is already present"),
            SystemError::HashCollision { existing, incoming } =>
                write!(f, "{incoming:?} hashes to the same id as {existing:?}"),
            SystemError::UnresolvedPrimary { body, primary } =>
                write!(f, "{body:?} orbits {primary:?}, which is not in the system"),
        }
    }
}
impl std::error::Error for SystemError {}

/// Everything needed to add a body.
#[derive(Clone)]
pub struct BodyDef {
    pub info: BodyInfo,
    pub motive: Motive,
    pub rotation: Option<BodyRotation>,
    /// How the body looks. Data, not rendering.
    pub appearance: Appearance,
}

/// The simulation arena.
pub struct System {
    // --- identity ---
    ids: Vec<BodyId>,
    info: Vec<BodyInfo>,
    index_of: HashMap<BodyId, BodyIndex>,

    // --- how each body moves ---
    motives: Vec<Motive>,
    rotations: Vec<Option<BodyRotation>>,
    appearance: Vec<Appearance>,

    // --- state written by propagation ---
    position: Vec<DVec3>,
    velocity: Vec<DVec3>,
    local_position: Vec<Option<DVec3>>,
    /// Set once a Newtonian body has been seeded; distinguishes seeded from not-yet-moved.
    newtonian_started: Vec<bool>,

    // --- derived, rebuilt when `dirty` ---
    parent: Vec<Option<BodyIndex>>,
    /// G(M_parent + M_body) for a Keplerian body; zero otherwise.
    mu: Vec<f64>,
    /// Hierarchical bodies (Fixed and Keplerian), parents before children.
    topo_order: Vec<BodyIndex>,
    newtonian: Vec<BodyIndex>,
    major: Vec<BodyIndex>,
    /// Bodies whose motive names a primary that is not in the arena, as (body, primary).
    /// Their mu falls back to `G * m_self` and their orbit anchors at the origin, so they
    /// still render and animate — this is the only record that they are wrong.
    unresolved: Vec<(String, String)>,

    gravitational_constant: f64,
    time: Instant,
    generation: u32,
    dirty: bool,
}

impl System {
    pub fn new(gravitational_constant: f64) -> Self {
        Self {
            ids: Vec::new(), info: Vec::new(), index_of: HashMap::new(),
            motives: Vec::new(), rotations: Vec::new(), appearance: Vec::new(),
            position: Vec::new(), velocity: Vec::new(), local_position: Vec::new(),
            newtonian_started: Vec::new(),
            parent: Vec::new(), mu: Vec::new(), topo_order: Vec::new(),
            newtonian: Vec::new(), major: Vec::new(), unresolved: Vec::new(),
            gravitational_constant, time: Instant::J2000, generation: 0, dirty: true,
        }
    }

    // ---------------------------------------------------------------- structure

    pub fn insert(&mut self, def: BodyDef) -> Result<BodyIndex, SystemError> {
        let id = BodyId::from_name(&def.info.id);
        if let Some(&existing) = self.index_of.get(&id) {
            let existing_name = self.info[existing.get()].id.clone();
            return Err(if existing_name == def.info.id {
                SystemError::DuplicateName(def.info.id)
            } else {
                SystemError::HashCollision { existing: existing_name, incoming: def.info.id }
            });
        }
        let slot = BodyIndex::new(self.ids.len() as u32);
        self.ids.push(id);
        self.info.push(def.info);
        self.motives.push(def.motive);
        self.rotations.push(def.rotation);
        self.appearance.push(def.appearance);
        self.position.push(DVec3::ZERO);
        self.velocity.push(DVec3::ZERO);
        self.local_position.push(None);
        self.newtonian_started.push(false);
        self.parent.push(None);
        self.mu.push(0.0);
        self.index_of.insert(id, slot);
        self.mark_dirty();
        Ok(slot)
    }

    /// Remove a body. **Invalidates every [`BodyIndex`]**: the last body is swapped into the
    /// freed slot and the generation advances.
    pub fn remove(&mut self, id: BodyId) -> bool {
        let Some(slot) = self.index_of.remove(&id) else { return false };
        let i = slot.get();
        let last = self.ids.len() - 1;
        for_each_column!(self, swap_remove, i);
        if i != last {
            // The body that was last now lives at `i`.
            self.index_of.insert(self.ids[i], BodyIndex::new(i as u32));
        }
        self.mark_dirty();
        true
    }

    /// Mark derived data stale and advance the generation.
    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.generation = self.generation.wrapping_add(1);
    }

    // ---------------------------------------------------------------- lookup

    #[inline]
    pub fn index_of(&self, id: BodyId) -> Option<BodyIndex> {
        self.index_of.get(&id).copied()
    }
    #[inline]
    pub fn by_name(&self, name: &str) -> Option<BodyIndex> {
        self.index_of(BodyId::from_name(name))
    }
    #[inline]
    pub fn len(&self) -> usize { self.ids.len() }
    #[inline]
    pub fn is_empty(&self) -> bool { self.ids.is_empty() }
    /// Advances whenever the set of bodies changes; a cached [`BodyIndex`] is valid only
    /// while it does not.
    #[inline]
    pub fn generation(&self) -> u32 { self.generation }
    #[inline]
    pub fn time(&self) -> Instant { self.time }
    #[inline]
    pub fn gravitational_constant(&self) -> f64 { self.gravitational_constant }

    #[inline]
    pub fn indices(&self) -> impl Iterator<Item = BodyIndex> {
        (0..self.ids.len() as u32).map(BodyIndex::new)
    }
    #[inline]
    pub fn iter_ids(&self) -> impl Iterator<Item = BodyId> + '_ {
        self.ids.iter().copied()
    }

    // ---------------------------------------------------------------- per body

    #[inline] pub fn id(&self, i: BodyIndex) -> BodyId { self.ids[i.get()] }
    #[inline] pub fn info(&self, i: BodyIndex) -> &BodyInfo { &self.info[i.get()] }
    #[inline] pub fn name(&self, i: BodyIndex) -> &str { &self.info[i.get()].id }
    #[inline] pub fn mass(&self, i: BodyIndex) -> f64 { self.info[i.get()].mass }
    #[inline] pub fn is_major(&self, i: BodyIndex) -> bool { self.info[i.get()].major }
    #[inline] pub fn motive(&self, i: BodyIndex) -> &Motive { &self.motives[i.get()] }
    #[inline] pub fn rotation(&self, i: BodyIndex) -> Option<&BodyRotation> {
        self.rotations[i.get()].as_ref()
    }
    #[inline] pub fn appearance(&self, i: BodyIndex) -> &Appearance { &self.appearance[i.get()] }
    /// Radius in metres, from the appearance.
    #[inline] pub fn radius(&self, i: BodyIndex) -> f64 { self.appearance[i.get()].radius() }
    #[inline] pub fn position(&self, i: BodyIndex) -> DVec3 { self.position[i.get()] }
    #[inline] pub fn velocity(&self, i: BodyIndex) -> DVec3 { self.velocity[i.get()] }
    /// Position relative to the primary, for a body that has one.
    #[inline] pub fn local_position(&self, i: BodyIndex) -> Option<DVec3> {
        self.local_position[i.get()]
    }
    #[inline] pub fn parent(&self, i: BodyIndex) -> Option<BodyIndex> { self.parent[i.get()] }
    /// G(M_primary + M_body) for a Keplerian body.
    #[inline] pub fn mu(&self, i: BodyIndex) -> f64 { self.mu[i.get()] }

    /// Bodies integrated rather than evaluated. Zero means any instant can be jumped to.
    #[inline] pub fn newtonian_count(&self) -> usize { self.newtonian.len() }

    /// Bodies naming a primary that is not present, as (body, primary).
    ///
    /// Such a body keeps propagating: its mu collapses to `G * m_self`, which is smaller
    /// than intended by the mass ratio, and its orbit anchors at the world origin instead
    /// of its primary. It still draws, so nothing else will report it. Only meaningful
    /// after a propagation, which is what rebuilds the list.
    pub fn unresolved_primaries(&self) -> &[(String, String)] { &self.unresolved }

    #[inline] pub fn positions(&self) -> &[DVec3] { &self.position }
    #[inline] pub fn velocities(&self) -> &[DVec3] { &self.velocity }

    /// Mutable motive access. Marks derived data stale: a motive can change the parent.
    pub fn motive_mut(&mut self, i: BodyIndex) -> &mut Motive {
        self.mark_dirty();
        &mut self.motives[i.get()]
    }

    /// Mutable static data. Marks derived data stale: mu of anything orbiting this body
    /// depends on its mass.
    pub fn info_mut(&mut self, i: BodyIndex) -> &mut BodyInfo {
        self.mark_dirty();
        &mut self.info[i.get()]
    }

    pub fn set_rotation(&mut self, i: BodyIndex, rotation: Option<BodyRotation>) {
        self.rotations[i.get()] = rotation;
    }

    // ------------------------------------------------- internals for `propagate`

    /// An edit has landed that the derived columns have not caught up with. Public so a
    /// caller can tell "edited since last step" from "clock did not move".
    pub fn is_dirty(&self) -> bool { self.dirty }
    pub(crate) fn set_time(&mut self, t: Instant) { self.time = t; }

    pub(crate) fn topo_order(&self) -> &[BodyIndex] { &self.topo_order }
    pub(crate) fn newtonian_indices(&self) -> &[BodyIndex] { &self.newtonian }
    pub(crate) fn major_indices(&self) -> &[BodyIndex] { &self.major }

    pub(crate) fn write_state(
        &mut self, i: BodyIndex, position: DVec3, velocity: DVec3, local: Option<DVec3>,
    ) {
        let s = i.get();
        self.position[s] = position;
        self.velocity[s] = velocity;
        self.local_position[s] = local;
    }
    pub(crate) fn newtonian_started(&self, i: BodyIndex) -> bool {
        self.newtonian_started[i.get()]
    }
    pub(crate) fn set_newtonian_started(&mut self, i: BodyIndex, started: bool) {
        self.newtonian_started[i.get()] = started;
    }

    /// Recompute parent links, gravitational parameters and traversal order.
    pub(crate) fn rebuild_derived(&mut self, time: Instant) {
        let n = self.ids.len();
        self.parent.clear(); self.parent.resize(n, None);
        self.mu.clear(); self.mu.resize(n, 0.0);
        self.topo_order.clear();
        self.newtonian.clear();
        self.major.clear();
        self.unresolved.clear();

        // Gather first, assign after: reading a motive borrows `self`.
        enum Kind { Fixed, Kepler(Option<f64>), Newton }
        let mut plan: Vec<(BodyIndex, Option<(BodyId, String)>, Kind)> = Vec::with_capacity(n);
        for i in self.indices() {
            let (_, selection) = self.motives[i.get()].motive_at(time);
            let (parent_name, kind) = match selection {
                MotiveSelection::Fixed { primary_id, .. } => (primary_id.as_deref(), Kind::Fixed),
                MotiveSelection::Keplerian(k) => (Some(k.primary_id.as_str()),
                                                  Kind::Kepler(k.gravitational_parameter)),
                MotiveSelection::Newtonian { .. } => (None, Kind::Newton),
            };
            plan.push((i, parent_name.map(|n| (BodyId::from_name(n), n.to_string())), kind));
        }

        let mut hierarchical = Vec::with_capacity(n);
        for (i, parent_ref, kind) in plan {
            if self.info[i.get()].major { self.major.push(i); }
            let parent = match &parent_ref {
                Some((pid, name)) => {
                    let found = self.index_of.get(pid).copied();
                    if found.is_none() {
                        self.unresolved.push((self.info[i.get()].id.clone(), name.clone()));
                    }
                    found
                }
                None => None,
            };
            self.parent[i.get()] = parent;
            match kind {
                Kind::Newton => self.newtonian.push(i),
                Kind::Kepler(explicit) => {
                    self.mu[i.get()] = match explicit {
                        // Barycentric orbits: effective mu is not G(M+m), the motive says it.
                        Some(mu) => mu,
                        None => {
                            let parent_mass = parent.map(|p| self.info[p.get()].mass).unwrap_or(0.0);
                            // Relative two-body motion: mu = G(M + m), not G*M.
                            self.gravitational_constant * (parent_mass + self.info[i.get()].mass)
                        }
                    };
                    hierarchical.push(i);
                }
                Kind::Fixed => hierarchical.push(i),
            }
        }
        self.topo_order = topological_order(&hierarchical, &self.parent);
        self.dirty = false;
    }
}

/// Order hierarchical bodies so every parent precedes its children. Cyclic or orphaned ones
/// are appended in arbitrary order rather than dropped.
fn topological_order(bodies: &[BodyIndex], parent: &[Option<BodyIndex>]) -> Vec<BodyIndex> {
    use std::collections::{HashSet, VecDeque};
    let member: HashSet<BodyIndex> = bodies.iter().copied().collect();
    let mut children: HashMap<BodyIndex, Vec<BodyIndex>> = HashMap::new();
    let mut queue: VecDeque<BodyIndex> = VecDeque::new();

    for &b in bodies {
        match parent[b.get()] {
            Some(p) if member.contains(&p) => children.entry(p).or_default().push(b),
            _ => queue.push_back(b), // root, or parented outside the hierarchy
        }
    }

    let mut out = Vec::with_capacity(bodies.len());
    let mut seen: HashSet<BodyIndex> = HashSet::with_capacity(bodies.len());
    while let Some(b) = queue.pop_front() {
        if !seen.insert(b) { continue; }
        out.push(b);
        if let Some(kids) = children.get(&b) {
            for &k in kids { queue.push_back(k); }
        }
    }
    for &b in bodies {
        if !seen.contains(&b) { out.push(b); }
    }
    out
}

/// Apply a `Vec` method to every per-body column, so a new column cannot fall out of sync.
macro_rules! for_each_column {
    ($self:ident, $op:ident, $($arg:expr),*) => {{
        $self.ids.$op($($arg),*);
        $self.info.$op($($arg),*);
        $self.motives.$op($($arg),*);
        $self.rotations.$op($($arg),*);
        $self.appearance.$op($($arg),*);
        $self.position.$op($($arg),*);
        $self.velocity.$op($($arg),*);
        $self.local_position.$op($($arg),*);
        $self.newtonian_started.$op($($arg),*);
        $self.parent.$op($($arg),*);
        $self.mu.$op($($arg),*);
    }};
}
use for_each_column;

// === Building a system from a save ===

impl System {
    /// Build a system from loaded universe contents. Legacy single-motive entries widen
    /// into a one-event [`Motive`] timeline.
    pub fn from_contents(contents: &crate::universe::UniverseFileContents) -> Result<Self, SystemError> {
        use crate::universe::SomeBody;
        let mut system = Self::new(contents.physics.gravitational_constant);
        for body in &contents.bodies {
            let (info, motive, rotation, appearance) = match body {
                SomeBody::FixedEntry(e) => (
                    e.info.clone(),
                    Motive::fixed_with_parent(e.info_primary(), e.position),
                    e.rotation.clone(), e.appearance.clone(),
                ),
                SomeBody::NewtonEntry(e) => (
                    e.info.clone(), Motive::newtonian(e.position, e.velocity), e.rotation.clone(),
                    e.appearance.clone(),
                ),
                SomeBody::KeplerEntry(e) => (
                    e.info.clone(),
                    Motive::from_keplerian(e.params.clone()),
                    e.rotation.clone(), e.appearance.clone(),
                ),
                SomeBody::CompoundMotiveEntry(e) => (
                    e.info.clone(), e.motive.clone(), e.rotation.clone(), e.appearance.clone(),
                ),
                // Deprecated: a route of Keplerian arcs. Take the first as a plain orbit.
                SomeBody::CompoundEntry(e) => {
                    let Some(first) = e.route.values().next() else { continue };
                    (e.info.clone(), Motive::from_keplerian(first.clone()), None, e.appearance.clone())
                }
            };
            system.insert(BodyDef { info, motive, rotation, appearance })?;
        }
        // A dangling primary is not survivable: mu collapses to G * m_self and the orbit
        // anchors at the origin, while the body keeps drawing as though it were fine.
        // Catch it here, where the file can still be rejected.
        system.rebuild_derived(system.time);
        let dangling = system.unresolved.first().cloned();
        // `rebuild_derived` clears the dirty flag, which would claim the arena is
        // evaluated at its epoch when no position has been computed yet — callers that
        // skip propagating an up-to-date system would then never place the bodies.
        system.dirty = true;
        if let Some((body, primary)) = dangling {
            return Err(SystemError::UnresolvedPrimary { body, primary });
        }

        Ok(system)
    }
}

impl System {
    /// Inverse of [`System::from_contents`]. Every body round-trips as a
    /// `CompoundMotiveEntry`. `time`, `physics` and `view` come from the app.
    pub fn to_contents(
        &self,
        time: crate::universe::UniverseFileTime,
        physics: crate::universe::UniversePhysics,
        view: crate::universe::ViewSettings,
    ) -> crate::universe::UniverseFileContents {
        use crate::universe::{CompoundMotiveEntry, SomeBody, UniverseFileContents};
        UniverseFileContents {
            version: "0.0".to_string(),
            time,
            view,
            physics,
            bodies: self
                .indices()
                .map(|i| {
                    SomeBody::CompoundMotiveEntry(CompoundMotiveEntry {
                        info: self.info[i.get()].clone(),
                        motive: self.motives[i.get()].clone(),
                        appearance: self.appearance[i.get()].clone(),
                        rotation: self.rotations[i.get()].clone(),
                    })
                })
                .collect(),
        }
    }
}
