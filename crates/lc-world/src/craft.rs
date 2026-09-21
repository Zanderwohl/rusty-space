//! A physical craft: something with a worldline, an instrument, and a name.
//!
//! [`motion`](crate::motion) says how a thing moves. This says what a thing *is*, so that the
//! same physics runs for a probe dropped into a ring, a relay parked at L2 and the ship the
//! player is sitting in. None of it was ever specific to the player; it was only ever reached
//! through them.
//!
//! [`Fleet`] is the single source of truth for craft state, in the same sense
//! `em_sim::System` is for body state: anything else that wants to draw or index a craft holds
//! its [`CraftId`] and looks it up. Nothing per-craft lives in two places.
//!
//! A craft owns the answer to "when does my arc leave this sphere", because solving it costs a
//! few hundred boundary evaluations and both the client and the server need exactly one copy.

use std::sync::Arc;

use glam::DVec3;

use crate::fitting::Fitting;
use crate::instrument::Instrument;
use crate::motion::{self, Event, Flight, Motive, Past, Rejected, ShipState};
use crate::system::LocalSystem;

/// A craft, by the identifier whoever owns it uses. Opaque here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CraftId(pub i64);

/// A hull's beam and height, as fractions of its length.
///
/// Every craft is the same ovoid at a different size: five long by three across by one deep.
/// Shape is not yet a thing a craft can differ in, so it is a constant rather than a field —
/// one that anything drawing or picking a hull reads, so the silhouette, the bounding size and
/// the zoom limits cannot drift apart.
pub const BEAM_PER_LENGTH: f64 = 3.0 / 5.0;
pub const HEIGHT_PER_LENGTH: f64 = 1.0 / 5.0;

/// How far back a craft remembers what it was doing, coordinate seconds.
///
/// The longest light delay anyone can solve across. [`crate::pursuit`] and the contact channel
/// both only look at craft inside the same shell, so two of them are at most `2 *
/// LOCAL_SHELL_LY` apart — and a light-year is a year of travel by definition, so that span in
/// years is that span in seconds of delay.
pub const HISTORY_S: f64 = 2.0 * crate::system::LOCAL_SHELL_LY * crate::flight::JULIAN_YEAR_S;

/// How many stretches of worldline a craft keeps at once.
///
/// A bound on the memory, and the reason [`Flight::defined_over`] has a near end at all. Most
/// craft have one or two: a ship holding a station has not changed what it is doing since it
/// arrived. A craft that maneuveres more often than this inside [`HISTORY_S`] forgets its
/// oldest stretches, and the observers far enough away to have wanted them stop seeing it —
/// which is the safe way to be unable to answer.
pub const HISTORY_STRETCHES: usize = 256;

/// What a hull sits at, kelvin.
///
/// One temperature for every craft, and a deliberate simplification: a real hull has a sunward
/// face and a shadowed one and a radiator problem that dominates its design. These are assumed
/// to be advanced enough to hold an even skin and dump exactly what they make, so a craft is a
/// blackbody at a single temperature and nothing about where it is or what it is doing changes
/// that.
///
/// Four hundred kelvin puts a ship well below anything visible and squarely in the thermal
/// infrared, which is the point: a craft running dark in the optical is a bright object at ten
/// microns, and which band a player is looking through decides whether they can see it.
pub const HULL_K: f64 = 400.0;

/// The span of hull lengths the game is designed around, meters. Nothing enforces it; it is
/// what the camera, the reticle and the point-source crossover are expected to cope with.
pub const LENGTH_RANGE_M: (f64, f64) = (500.0, 50_000.0);

/// What a craft is for.
///
/// It decides the defaults and what the interface offers, never the physics: a probe on a
/// hyperbola and a ship on the same hyperbola are the same arc.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    /// Crewed. The only kind with an acceleration limit set by what a body can stand.
    #[default]
    Ship,
    /// Sent somewhere to look at it and say what it saw.
    Probe,
    /// Parked to pass messages on. Its whole purpose is to be somewhere with a line of sight
    /// to two places that have none to each other.
    Relay,
    /// Put somewhere to be seen. Transmits, never listens.
    Beacon,
}

impl Kind {
    /// What it can pull, and how fast it will go.
    ///
    /// A crew caps acceleration and nothing else does; an uncrewed craft is limited by its
    /// structure, which is a long way above anything a person survives.
    ///
    /// The slew rate is the *default* hull's, because that is all a kind knows. A craft built to
    /// another size turns at another rate, and [`Craft::turning`] is where that is put right.
    pub fn drive(self) -> crate::flight::Drive {
        let base = match self {
            Kind::Ship => crate::flight::Drive::DEFAULT,
            Kind::Probe => {
                crate::flight::Drive { accel_g: 30.0, ..crate::flight::Drive::DEFAULT }
            }
            // Neither of these is going anywhere in a hurry once it is placed, and neither
            // carries a torch to do it with — a station-keeping thruster throws mass at a
            // few hundred kilometers a second, so a beacon correcting itself is a thing you
            // would have to be close to see.
            Kind::Relay | Kind::Beacon => crate::flight::Drive {
                accel_g: 1.0,
                max_beta: 0.9,
                exhaust_v_m_s: 0.002 * crate::flight::C_M_S,
                ..crate::flight::Drive::DEFAULT
            },
        };
        crate::flight::Drive {
            slew_rate_rad_s: crate::attitude::rate_rad_s(self.length_m()),
            ..base
        }
    }

    /// How long a hull of this kind is by default, meters. A craft may be given another.
    pub fn length_m(self) -> f64 {
        match self {
            Kind::Ship => 500.0,
            Kind::Probe => 40.0,
            Kind::Relay => 120.0,
            Kind::Beacon => 20.0,
        }
    }

    /// What a hull of this kind averages over its whole volume, kilograms per cubic meter.
    ///
    /// An average and not a material: most of a craft is empty, and what fills the rest differs
    /// by what the craft is for. A crewed ship carries decks, tankage and shielding; a probe is
    /// dense instrument and no room to stand up in; a relay and a beacon are mostly a shape
    /// holding an antenna apart from itself.
    ///
    /// Water is a thousand and a modern warship is about two hundred, which is the number to
    /// argue with. It is here rather than as a mass because mass has to follow size — see
    /// [`Craft::mass_kg`] — and two ships of a kind are allowed to be different sizes.
    pub fn density_kg_m3(self) -> f64 {
        match self {
            Kind::Ship => 250.0,
            Kind::Probe => 400.0,
            Kind::Relay => 150.0,
            Kind::Beacon => 100.0,
        }
    }

    /// What a craft of this kind carries to look with. A beacon carries nothing.
    pub fn sensor(self) -> Instrument {
        match self {
            Kind::Ship => Instrument::SHIP,
            Kind::Probe => Instrument::PROBE,
            Kind::Relay | Kind::Beacon => Instrument::PROBE,
        }
    }
}

/// One craft: a worldline, an instrument, and who it is.
#[derive(Clone)]
pub struct Craft {
    pub id: CraftId,
    pub kind: Kind,
    /// What a player calls it. `None` until someone does.
    pub name: Option<String>,
    pub motion: ShipState,
    /// How long the hull is, meters. Defaults to the kind's, and is a field rather than a
    /// lookup because two ships of one kind are allowed to be different sizes — see
    /// [`LENGTH_RANGE_M`], which is the span the camera and the reticle are built for.
    pub length_m: f64,
    /// The system its motive is defined against, if it is in one.
    ///
    /// Shared and never mutated: every motive is evaluated at the time asked for rather than
    /// read out of a propagated arena, so nothing has to advance a system and every craft in
    /// one can point at the same copy.
    pub system: Option<Arc<LocalSystem>>,
    pub sensor: Instrument,
    /// Below this, an arrival is not a detection.
    pub noise_floor: f32,
    /// The patch the current arc is heading for, solved once when the arc began.
    ///
    /// Private because it is derived: the arc is the fact and this is an answer about it, and
    /// an answer that can be set from outside is an answer that can be about a different arc.
    patch: Option<Event>,
    /// What it was doing before, oldest first. See [`Flight`].
    ///
    /// Private for a stronger reason than the patch: it is only right if *every* change to the
    /// motive appends to it, so the appending lives in [`Craft::remembering`] and there is no
    /// way to change a motive that goes round it.
    past: Vec<Past>,
    /// The earliest coordinate second this craft can answer for.
    known_from_s: f64,
    /// Modules and energy. Only a player's ship has one; see [`crate::fitting`].
    ///
    /// Private for the reason `past` is: every change of motive has to settle it, and that
    /// happens in [`Craft::remembering`].
    fitting: Option<Fitting>,
}

impl Craft {
    /// A craft of `kind`, at rest at a point, light-years from the world origin.
    pub fn at(id: CraftId, kind: Kind, position_ly: DVec3) -> Self {
        let mut motion = ShipState::at(position_ly);
        motion.drive = kind.drive();
        Self {
            id,
            kind,
            name: None,
            motion,
            length_m: kind.length_m(),
            system: None,
            sensor: kind.sensor(),
            noise_floor: 0.0,
            patch: None,
            past: Vec::new(),
            // Nothing has been forgotten, because nothing has happened. A craft that has never
            // changed what it is doing has always been doing it, which is as true a statement
            // about its past as there is.
            known_from_s: f64::NEG_INFINITY,
            fitting: None,
        }
    }

    /// Do something that may change what this craft is doing, and remember what it was.
    ///
    /// **Every** change to the motive goes through here. The alternative is recording at each
    /// call site, and the transitions inside [`motion::advance`] — a crossing arriving, an arc
    /// losing its system — happen without any call site knowing they did.
    ///
    /// The clone is the cost of that, and it is paid once per change rather than once per
    /// step: the comparison is what decides, and a craft that went on doing what it was doing
    /// keeps its history exactly as it was.
    fn remembering<T>(&mut self, at_s: f64, change: impl FnOnce(&mut Self) -> T) -> T {
        let before = self.motion.clone();
        // Where the turn the old motive had ordered got to. The new one starts from there,
        // because a ship does not snap back to where it was pointing when it was given a new
        // order — this is the one place the two motives are both in hand, so it is the only
        // place that hand-over can happen.
        let began = self.past.last().map_or(f64::NEG_INFINITY, |entry| entry.until_s);
        let nose = self.facing_of(&before, began, at_s);
        // **The plan is made from where the nose is, not from where the last order left it.**
        // A plan holds its start attitude as a parameter and is re-planned from it on the far
        // side of a wire or a checkpoint — see [`crate::resume`] — and what is re-planned from
        // is this field. Setting it after the plan was made left the two disagreeing: the live
        // crossing turned from the old attitude and the restored one from the new, so a ship
        // came back from a checkpoint on a slightly different flight. It only showed once an
        // idle ship's nose could move on its own, which is what turning broadside does.
        let previous = std::mem::replace(&mut self.motion.attitude, nose);
        let out = change(self);
        if before.same_worldline_as(&self.motion) {
            // Nothing happened, so nothing may be recorded — including the attitude, which
            // would otherwise creep forward on every step and restart every turn from where it
            // had got to.
            self.motion.attitude = previous;
        } else {
            if let Some(fitting) = &mut self.fitting {
                fitting.settle(&before, at_s);
                fitting.commit(&self.motion, at_s);
            }
            // A new motive may collect where the old one could not, or stop collecting.
            self.begin_solar_segment(at_s);
            self.past.push(Past { until_s: at_s, motion: before });
            self.forget_before(at_s);
        }
        out
    }

    /// Drop what is too old or too much to keep, and record how far back that leaves.
    fn forget_before(&mut self, now_s: f64) {
        let horizon = now_s - HISTORY_S;
        let stale = self.past.iter().take_while(|entry| entry.until_s < horizon).count();
        let excess = self.past.len().saturating_sub(HISTORY_STRETCHES);
        let drop = stale.max(excess);
        if drop == 0 {
            return;
        }
        // The newest one dropped is the boundary: everything before it is a stretch this craft
        // can no longer name, so it may no longer be asked about.
        self.known_from_s = self.past[drop - 1].until_s;
        self.past.drain(..drop);
    }

    /// How many stretches of its past this craft is holding. For tests and diagnostics.
    pub fn remembered(&self) -> usize {
        self.past.len()
    }

    /// The earliest coordinate second this craft can be asked about.
    pub fn known_from_s(&self) -> f64 {
        self.known_from_s
    }

    /// What to call it on screen: its name, or its kind and number.
    pub fn designation(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("{:?} {}", self.kind, self.id.0),
        }
    }

    /// What it was doing at a coordinate second, as far back as it remembers.
    pub fn motion_at(&self, s: f64) -> &ShipState {
        self.past.iter().find(|entry| s < entry.until_s).map(|entry| &entry.motion).unwrap_or(&self.motion)
    }

    /// What it has done, oldest first: each motive with the coordinate second it stopped, and
    /// the current one with infinity.
    pub fn stretches(&self) -> impl Iterator<Item = (f64, &ShipState)> + '_ {
        self.past
            .iter()
            .map(|entry| (entry.until_s, &entry.motion))
            .chain(std::iter::once((f64::INFINITY, &self.motion)))
    }

    /// The craft as something a light-delay solve can evaluate.
    pub fn worldline(&self) -> Flight<'_> {
        Flight::with_past(&self.motion, self.system.as_deref(), &self.past, self.known_from_s)
    }

    /// How fast it can turn, radians a second. See [`crate::attitude`].
    pub fn slew_rate_rad_s(&self) -> f64 {
        crate::attitude::rate_rad_s(self.length_m)
    }

    /// `drive`, turning at *this hull's* rate rather than at whatever was stamped on it.
    ///
    /// Every maneuvere is planned through here, because the slew rate is a plan parameter — a
    /// crossing holds its coast open for the flip — and the only honest source for it is the
    /// hull that is flying. Stamped on rather than stored, for the reason [`Craft::mass_kg`]
    /// gives: a kept copy is a copy that can disagree with the ship it belongs to, and
    /// [`Craft::length_m`] is a field anyone may set.
    ///
    /// Which engine to hand it is the caller's business and the two answers differ. `kind.drive()`
    /// is the *ceiling*, which is what a new order is clamped against; `motion.drive` is what the
    /// ship is flying with now, which is what a standing policy should go on flying with.
    pub fn turning(&self, drive: crate::flight::Drive) -> crate::flight::Drive {
        crate::flight::Drive { slew_rate_rad_s: self.slew_rate_rad_s(), ..drive }
    }

    /// How much hull there is, cubic meters.
    ///
    /// The ovoid [`BEAM_PER_LENGTH`] and its neighbor describe, so a craft's volume follows
    /// from the one number that says how big it is. Cubic in the length: a fifty-kilometer ship
    /// is a million times the ship a five-hundred-meter one is, which is worth knowing before
    /// being surprised by what it weighs.
    pub fn volume_m3(&self) -> f64 {
        let half = self.length_m * 0.5;
        4.0 / 3.0
            * std::f64::consts::PI
            * half
            * (half * BEAM_PER_LENGTH)
            * (half * HEIGHT_PER_LENGTH)
    }

    /// What it weighs at a coordinate second, kilograms: its modules and its stored energy if it
    /// has a fitting, and [`Craft::mass_kg`] if it does not.
    pub fn mass_kg_at(&self, now_s: f64) -> f64 {
        match &self.fitting {
            Some(fitting) => fitting.mass_kg_at(&self.motion, now_s),
            None => self.mass_kg(),
        }
    }

    /// What an unfitted craft weighs, kilograms.
    ///
    /// Size times what that size is made of. Nothing stores a mass, because a stored one could
    /// disagree with the hull it belongs to — and everything that wants a mass wants it to
    /// follow the ship being talked about.
    pub fn mass_kg(&self) -> f64 {
        self.kind.density_kg_m3() * self.volume_m3()
    }

    /// What the drive is putting into its exhaust at a coordinate second, watts.
    ///
    /// Zero whenever nothing is lit, which is most of the time: a ship coasts far more than it
    /// burns. Everything visible about a burn is this number — see [`crate::flight::Drive`].
    pub fn jet_power_w(&self, now_s: f64) -> f64 {
        let accel_g = motion::thrust_g(&self.motion, now_s);
        if accel_g <= 0.0 {
            return 0.0;
        }
        self.motion.drive.jet_power_w(self.mass_kg_at(now_s), accel_g)
    }

    /// Which way the nose points at a coordinate second, or `None` when nothing decides it.
    pub fn facing_at(&self, now_s: f64) -> Option<DVec3> {
        let began = self.past.last().map_or(f64::NEG_INFINITY, |entry| entry.until_s);
        Some(self.facing_of(&self.motion, began, now_s))
    }

    /// Where `state`'s nose points at `now_s`, for a motive that began at `began_s`.
    ///
    /// A plan points the nose where it needs it. A fitted craft with no plan, inside a system,
    /// turns broadside to the star so its collectors face it — see
    /// `lightcone/docs/20-solar-power.md` — swinging from the attitude its last order left it at,
    /// at its hull's rate. The nose goes perpendicular to the star by the smallest turn; the
    /// renderer rolls the belly toward the star about it.
    fn facing_of(&self, state: &ShipState, began_s: f64, now_s: f64) -> DVec3 {
        let broadside = (self.fitting.is_some() && !state.is_under_way())
            .then(|| self.to_star(state, now_s))
            .flatten()
            .map(|to_star| broadside_nose(state.attitude, to_star));
        match broadside {
            Some(to) => crate::attitude::turned(state.attitude, to, self.slew_rate_rad_s(), now_s - began_s),
            None => motion::facing_at(state, self.length_m, now_s),
        }
    }

    /// Unit vector from `state`'s position toward its system's primary at `t`.
    fn to_star(&self, state: &ShipState, t: f64) -> Option<DVec3> {
        let system = self.system.as_deref()?;
        let star = system.star_position_at(t)?;
        let (at, _) = motion::state_at(state, Some(system), t)?;
        Some((star - at).normalize_or_zero()).filter(|d| *d != DVec3::ZERO)
    }

    /// Where it is at a coordinate microsecond, light-microseconds from the world origin.
    pub fn position_at(&self, t_us: f64) -> DVec3 {
        use lc_spacetime::Worldline;
        self.worldline().position_at(t_us)
    }

    pub fn fitting(&self) -> Option<&Fitting> {
        self.fitting.as_ref()
    }

    /// Give it modules, or take them away. Its length follows its slots from here on.
    pub fn fit(&mut self, fitting: Option<Fitting>) {
        self.fitting = fitting;
        self.sync_length();
        if let Some(since) = self.fitting.as_ref().map(Fitting::since_s) {
            self.begin_solar_segment(since);
        }
    }

    /// How far it is from its system's primary at `t`, meters. `None` between systems.
    pub fn star_distance_m_at(&self, t: f64) -> Option<f64> {
        let system = self.system.as_deref()?;
        let star = system.star_position_at(t)?;
        let (at, _) = motion::state_at(&self.motion, Some(system), t)?;
        Some((at - star).length() * crate::system::M_PER_LY)
    }

    /// What its hull would collect broadside at `t`, watts, at this length. Zero under way,
    /// between systems, and for a craft with no fitting.
    pub fn solar_w_at(&self, t: f64) -> f64 {
        self.solar_w_for(self.length_m, t)
    }

    /// [`Craft::solar_w_at`] for a hull of another length, as a refit would leave it.
    pub fn solar_w_for(&self, length_m: f64, t: f64) -> f64 {
        let (Some(fitting), Some(system)) = (&self.fitting, self.system.as_deref()) else {
            return 0.0;
        };
        if self.motion.is_under_way() {
            return 0.0;
        }
        let Some(distance_m) = self.star_distance_m_at(t) else { return 0.0 };
        crate::solar::power_w(&fitting.balance, length_m, system.star_luminosity_w(), distance_m)
    }

    /// Start the income segment that begins at `from_s`, at the power collected at its midpoint.
    /// Midpoint rather than start, so a distance that changes across the segment averages out to
    /// first order. Settles to `from_s` first, so the old power is not applied backwards.
    fn begin_solar_segment(&mut self, from_s: f64) {
        let Some(since) = self.fitting.as_ref().map(Fitting::since_s) else { return };
        let from_s = from_s.max(since);
        if let Some(fitting) = &mut self.fitting {
            fitting.settle(&self.motion, from_s);
        }
        let middle = 0.5 * (from_s + crate::solar::segment_end(from_s));
        let watts = self.solar_w_at(middle);
        if let Some(fitting) = &mut self.fitting {
            fitting.set_solar_w(watts);
        }
    }

    /// Settle at every income boundary up to `now_s`, starting each new segment as it goes.
    fn collect_to(&mut self, now_s: f64) {
        let Some(since) = self.fitting.as_ref().map(Fitting::since_s) else { return };
        let step = crate::solar::step_for(since, now_s);
        let mut boundary = ((since / step).floor() + 1.0) * step;
        while boundary <= now_s {
            if let Some(fitting) = &mut self.fitting {
                fitting.settle(&self.motion, boundary);
            }
            let middle = boundary + 0.5 * step;
            let watts = self.solar_w_at(middle);
            if let Some(fitting) = &mut self.fitting {
                fitting.set_solar_w(watts);
            }
            boundary += step;
        }
    }

    fn sync_length(&mut self) {
        if let Some(fitting) = &self.fitting {
            self.length_m = fitting.balance.length_m(fitting.loadout.slots);
        }
    }

    /// Fold the fitting's account up to `now_s`: finished refit steps, drain, and whatever the
    /// motive has burned.
    pub fn settle(&mut self, now_s: f64) {
        self.collect_to(now_s);
        if let Some(fitting) = &mut self.fitting {
            fitting.settle(&self.motion, now_s);
        }
        self.sync_length();
    }

    /// The drive a new order is clamped against: the kind's, turning at this hull's rate, and
    /// pulling what its engines can move this mass at if it has engines.
    pub fn rated_drive(&self, now_s: f64) -> crate::flight::Drive {
        let mut drive = self.turning(self.kind.drive());
        if let Some(fitting) = &self.fitting {
            drive.accel_g = fitting.rated_g_at(&self.motion, now_s);
        }
        drive
    }

    /// Energy stored and not committed, joules. Unlimited for a craft with no fitting.
    pub fn free_j_at(&self, now_s: f64) -> f64 {
        match &self.fitting {
            Some(fitting) => fitting.free_j_at(&self.motion, now_s),
            None => f64::INFINITY,
        }
    }

    /// Whether a refit is under way at `now_s`. Flying and refitting exclude each other.
    pub fn is_refitting(&self, now_s: f64) -> bool {
        self.fitting.as_ref().and_then(Fitting::refit).is_some_and(|r| !r.is_done(now_s))
    }

    /// What folding `event` would commit, joules, without folding it. Zero for an unfitted craft.
    ///
    /// The plan's whole remaining rapidity at this craft's mass now. [`Craft::apply`] commits the
    /// same number, because it is the same arithmetic on the same state.
    pub fn cost_of(&self, event: &Event) -> Result<f64, Rejected> {
        let Some(fitting) = &self.fitting else { return Ok(0.0) };
        let mut trial = self.motion.clone();
        motion::apply(&mut trial, self.system.as_deref(), event)?;
        let mut account = fitting.clone();
        account.settle(&self.motion, event.at_t);
        Ok(account.commit(&trial, event.at_t))
    }

    /// Add energy, as far as storage allows.
    pub fn grant(&mut self, joules: f64, now_s: f64) {
        self.settle(now_s);
        if let Some(fitting) = &mut self.fitting {
            fitting.grant(joules);
        }
    }

    /// Begin rebuilding towards `target`. Refused while under way, and when it cannot be done.
    pub fn begin_refit(
        &mut self,
        target: crate::fitting::Loadout,
        now_s: f64,
    ) -> Result<(), crate::refit::Shortage> {
        self.settle(now_s);
        let Some(fitting) = &mut self.fitting else {
            return Err(crate::refit::Shortage::NoDrones);
        };
        let order = crate::refit::Order {
            from: fitting.loadout,
            target,
            stored_j: fitting.stored_j_at(&self.motion, now_s),
            start_s: now_s,
        };
        let refit = order.solve(&fitting.balance)?;
        fitting.begin_refit(refit);
        Ok(())
    }

    /// Stop a refit where it is, reversing the step in progress.
    pub fn cancel_refit(&mut self, now_s: f64) {
        self.settle(now_s);
        if let Some(fitting) = &mut self.fitting {
            fitting.cancel_refit(now_s);
        }
        self.sync_length();
    }

    /// Cut to a straight line from a point, at a velocity. What a burn does.
    ///
    /// A method rather than something a caller assembles, because the *only* way a worldline
    /// may change is through [`Craft::remembering`] — the server used to replace the whole
    /// craft here, which both lost everything that was not motion and left no trace of what it
    /// had been doing.
    pub fn drift_from(&mut self, at_ly: DVec3, beta: DVec3, now_s: f64) {
        if let Some(fitting) = &mut self.fitting {
            let was = motion::state_at(&self.motion, self.system.as_deref(), now_s)
                .map_or(self.motion.beta, |(_, beta)| beta);
            fitting.settle(&self.motion, now_s);
            fitting.spend(crate::cost::rapidity_between(was, beta));
        }
        self.remembering(now_s, |craft| {
            craft.motion.position_ly = at_ly;
            craft.motion.beta = beta;
            craft.motion.set_adrift(now_s);
            craft.solve_patch(now_s);
        });
    }

    /// Put it on an approach, and drop whatever the old motive had predicted.
    ///
    /// Not an [`Event`], because an approach is not an order a client sends: it is what the
    /// authority works out *from* a standing order, once per re-solve, against a sighting only
    /// it can vouch for. The client receives the answer as a motive and folds it.
    pub fn begin_escort(&mut self, plan: crate::escort::Escort, now_s: f64) {
        self.remembering(now_s, |craft| {
            craft.motion.begin_escort(plan);
            craft.solve_patch(now_s);
        });
    }

    pub fn begin_consort(&mut self, plan: crate::consort::Consort, now_s: f64) {
        self.remembering(now_s, |craft| {
            craft.motion.begin_consort(plan);
            craft.solve_patch(now_s);
        });
    }

    pub fn begin_rendezvous(&mut self, plan: crate::pursuit::Rendezvous, now_s: f64) {
        self.remembering(now_s, |craft| {
            craft.motion.begin_rendezvous(plan);
            craft.solve_patch(now_s);
        });
    }

    /// Fold an event, and re-solve the patch if the arc changed.
    pub fn apply(&mut self, event: &Event) -> Result<(), Rejected> {
        self.remembering(event.at_t, |craft| {
            motion::apply(&mut craft.motion, craft.system.as_deref(), event)?;
            craft.solve_patch(event.at_t);
            Ok(())
        })
    }

    /// Put it in a system, or take it out of one. Anything defined against the old system's
    /// bodies goes; a crossing does not, because leaving is what one is for.
    pub fn enter(&mut self, system: Option<Arc<LocalSystem>>, now_s: f64) {
        // Only on the way *out*. A craft that had no system has nothing defined against one,
        // so entering must leave its motive alone -- dropping it there canceled a station the
        // caller had just set for the system it was being put into.
        let left = match (&self.system, &system) {
            (None, _) => false,
            (Some(old), Some(new)) => !Arc::ptr_eq(old, new),
            (Some(_), None) => true,
        };
        let changed = match (&self.system, &system) {
            (Some(old), Some(new)) => !Arc::ptr_eq(old, new),
            (None, None) => false,
            _ => true,
        };
        self.remembering(now_s, |craft| {
            craft.system = system;
            if left {
                craft.motion.leave_system(now_s);
            }
            craft.solve_patch(now_s);
        });
        // A star to collect from, or none, from here.
        if changed {
            self.begin_solar_segment(now_s);
        }
    }

    /// Move to a coordinate time, folding the patch its arc was solved for if that time has
    /// come.
    ///
    /// The patch is folded *before* the step rather than after, so it is always stamped with
    /// its own solved coordinate whatever step happened to run past it. That is what lets a
    /// server at 438 seconds and a client at 61 reach the same arc.
    pub fn advance(&mut self, now_s: f64, elapsed_s: f64) {
        // Before anything changes the motive: each segment passed is priced by the motive that
        // was flying it.
        self.collect_to(now_s);
        self.sync_length();
        self.patch_if_due(now_s);
        self.remembering(now_s, |craft| {
            let was = matches!(craft.motion.motive, Motive::Falling(_));
            motion::advance(&mut craft.motion, craft.system.as_deref(), now_s, elapsed_s);
            // A crossing that arrived, or an arc that lost its system: either way the answer
            // the patch held is about a motive the craft is no longer on.
            if was != matches!(craft.motion.motive, Motive::Falling(_)) {
                craft.solve_patch(now_s);
            }
        });
        // A finished refit step changes the loadout, and a grown hull its length.
        if self.fitting.as_ref().is_some_and(|f| f.refit().is_some()) {
            self.settle(now_s);
        }
    }

    /// When the current arc leaves the sphere it was solved in, if it does.
    pub fn patch_due_at(&self) -> Option<f64> {
        self.patch.as_ref().map(|event| event.at_t)
    }

    /// Solve for the next patch, for whoever is authoritative over this craft.
    fn solve_patch(&mut self, from_s: f64) {
        self.patch = match (&self.motion.motive, self.system.as_deref()) {
            (Motive::Falling(_), Some(system)) => {
                motion::repatch_at(&self.motion, system, from_s)
            }
            // A prediction is about one conic. Anything else -- a course set, a station held,
            // a shell crossed -- makes it an answer to a question nobody is asking, and
            // folding it would cut the drive on a flight that is under way.
            _ => None,
        };
    }

    fn patch_if_due(&mut self, now_s: f64) {
        let Some(system) = self.system.clone() else {
            self.patch = None;
            return;
        };
        if !matches!(self.motion.motive, Motive::Falling(_)) {
            self.patch = None;
            return;
        }
        let event = match self.patch.take_if(|e| e.at_t <= now_s) {
            Some(solved) => Some(solved),
            // The backstop: a craft that ended up in a sphere the prediction did not cover,
            // which a course change or a system reload can do.
            None => motion::repatch_due(&self.motion, &system, now_s),
        };
        if let Some(event) = event {
            // Stamped at the patch's own coordinate and not at `now_s`, so the stretch it ends
            // is recorded as ending where the conic actually changed primary. A step that ran
            // past the join would otherwise claim the old arc held until the end of the step.
            self.remembering(event.at_t, |craft| {
                let _ = motion::apply(&mut craft.motion, Some(&system), &event);
                craft.solve_patch(event.at_t);
            });
        }
    }
}

impl std::fmt::Debug for Craft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Craft")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("motion", &self.motion)
            .field("in_a_system", &self.system.is_some())
            .finish()
    }
}

/// The nose direction nearest `attitude` that is perpendicular to `to_star`: `attitude` with its
/// component along the star removed. A nose pointing straight at the star or away from it has no
/// nearest perpendicular, and takes the one toward ecliptic north.
pub fn broadside_nose(attitude: DVec3, to_star: DVec3) -> DVec3 {
    let s = to_star.normalize_or_zero();
    let across = attitude - s * attitude.dot(s);
    if across.length_squared() > 1.0e-12 {
        return across.normalize();
    }
    let reference = if s.z.abs() > 0.999 { DVec3::X } else { DVec3::Z };
    (reference - s * reference.dot(s)).normalize_or_zero()
}

/// Every craft there is. The single source of truth for craft state.
#[derive(Clone, Debug, Default)]
pub struct Fleet {
    craft: Vec<Craft>,
}

impl Fleet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a craft, replacing any with the same identifier.
    pub fn insert(&mut self, craft: Craft) {
        match self.craft.iter_mut().find(|c| c.id == craft.id) {
            Some(existing) => *existing = craft,
            None => self.craft.push(craft),
        }
    }

    pub fn remove(&mut self, id: CraftId) -> Option<Craft> {
        let at = self.craft.iter().position(|c| c.id == id)?;
        Some(self.craft.remove(at))
    }

    pub fn get(&self, id: CraftId) -> Option<&Craft> {
        self.craft.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: CraftId) -> Option<&mut Craft> {
        self.craft.iter_mut().find(|c| c.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Craft> {
        self.craft.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Craft> {
        self.craft.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.craft.len()
    }

    pub fn is_empty(&self) -> bool {
        self.craft.is_empty()
    }

    /// Route an event to the craft it happened to.
    ///
    /// `None` when there is no such craft, which is not an error: a client can be told about
    /// something that happened to a craft it has never heard of.
    pub fn apply(&mut self, id: CraftId, event: &Event) -> Option<Result<(), Rejected>> {
        Some(self.get_mut(id)?.apply(event))
    }

    /// Move every craft to a coordinate time.
    pub fn advance(&mut self, now_s: f64, elapsed_s: f64) {
        for craft in &mut self.craft {
            craft.advance(now_s, elapsed_s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{Change, ShipId};
    use crate::navigation::{Course, Plane};
    use crate::sky::{CatalogueStar, StarProvider};

    fn sol() -> Option<Arc<LocalSystem>> {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .ok()?;
        let sun: CatalogueStar =
            provider.stars().iter().find(|s| s.provenance.name.as_deref() == Some("Sol"))?.clone();
        let mut system = LocalSystem::for_star(&sun)?;
        system.advance_to(0.0);
        Some(Arc::new(system))
    }

    fn escaping(system: &LocalSystem) -> (DVec3, DVec3) {
        let earth = system.body_named("Earth").expect("Earth");
        let (at_m, carried) = system.body_state_at(earth, 0.0).expect("a state");
        let radius = system.sim().radius(earth) * 3.0;
        let mu = system.sim().gravitational_constant() * system.sim().mass(earth);
        (
            system.origin_ly + (at_m + DVec3::X * radius) / crate::system::M_PER_LY,
            carried + DVec3::Y * (mu / radius).sqrt() * 1.6,
        )
    }

    /// The whole point: the physics does not know or care which craft it is running for.
    /// A probe and a ship given the same course fly the same arc.
    #[test]
    fn a_probe_and_a_ship_fly_the_same_arc() {
        let Some(system) = sol() else { return };
        let course = Change::SetCourse {
            course: Course::Orbit {
                body: "Earth".into(),
                altitude_radii: 2.0,
                plane: Plane::Equatorial,
            },
            // The same drive, because the *kind* changes what a craft can pull and the
            // physics changes nothing.
            drive: Kind::Ship.drive(),
        };

        let mut fleet = Fleet::new();
        for (id, kind) in [(1, Kind::Ship), (2, Kind::Probe)] {
            let mut craft = Craft::at(CraftId(id), kind, DVec3::ZERO);
            craft.enter(Some(system.clone()), 0.0);
            fleet.insert(craft);
        }
        for id in [1, 2] {
            let event = Event { ship: ShipId(id), at_t: 0.0, change: course.clone() };
            fleet.apply(CraftId(id), &event).expect("a craft").expect("a course");
        }
        fleet.advance(30_000.0, 30_000.0);

        let ship = fleet.get(CraftId(1)).expect("the ship");
        let probe = fleet.get(CraftId(2)).expect("the probe");
        assert_eq!(ship.motion.position_ly, probe.motion.position_ly);
        assert_eq!(ship.motion.motive, probe.motion.motive);
        // And they are still different things.
        assert_ne!(ship.kind, probe.kind);
        assert!(probe.sensor.aperture_m2 < ship.sensor.aperture_m2);
        assert!(probe.kind.drive().accel_g > ship.kind.drive().accel_g);
    }

    /// A craft owns its own patch, so a fleet of them all get one without anyone outside
    /// keeping a table of arcs.
    #[test]
    fn every_craft_solves_its_own_patch() {
        let Some(system) = sol() else { return };
        let (at, velocity) = escaping(&system);

        let mut fleet = Fleet::new();
        for id in 1..=3 {
            let mut craft = Craft::at(CraftId(id), Kind::Probe, at);
            craft.enter(Some(system.clone()), 0.0);
            // Before the cut: a craft at rest is falling straight down the line to its
            // primary, which has no elements, and cutting there leaves it drifting.
            craft.motion.beta = crate::coast::beta_of(velocity);
            craft
                .apply(&Event { ship: ShipId(id), at_t: 0.0, change: Change::CutDrive })
                .expect("the engine cuts");
            fleet.insert(craft);
        }

        let due: Vec<Option<f64>> = fleet.iter().map(|c| c.patch_due_at()).collect();
        assert!(due.iter().all(|d| d.is_some()), "{due:?}");
        assert!(due.windows(2).all(|w| w[0] == w[1]), "the same arc gave different answers");

        // Run past it: every one of them changes primary, and none needed telling.
        let patch = due[0].expect("a patch");
        fleet.advance(patch + 3_600.0, patch + 3_600.0);
        for craft in fleet.iter() {
            let Motive::Falling(arc) = &craft.motion.motive else {
                panic!("{:?}", craft.motion.motive)
            };
            assert_ne!(arc.primary, "Earth", "craft {:?} never left", craft.id);
        }
    }

    /// Leaving a system drops what was defined against it, and the patch with it.
    #[test]
    fn leaving_a_system_drops_the_arc_and_its_patch() {
        let Some(system) = sol() else { return };
        let (at, velocity) = escaping(&system);
        let mut craft = Craft::at(CraftId(1), Kind::Relay, at);
        craft.enter(Some(system.clone()), 0.0);
        craft.motion.beta = crate::coast::beta_of(velocity);
        craft.apply(&Event { ship: ShipId(1), at_t: 0.0, change: Change::CutDrive }).unwrap();
        assert!(craft.patch_due_at().is_some());

        craft.enter(None, 0.0);
        assert!(craft.patch_due_at().is_none(), "a patch about a system that is not there");
        assert!(matches!(craft.motion.motive, Motive::Drifting { .. }), "{:?}", craft.motion.motive);
    }

    /// A fleet is a set: one entry per identifier, and inserting the same one twice replaces
    /// rather than duplicates.
    #[test]
    fn a_fleet_holds_one_of_each() {
        let mut fleet = Fleet::new();
        fleet.insert(Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO));
        fleet.insert(Craft::at(CraftId(1), Kind::Beacon, DVec3::X));
        assert_eq!(fleet.len(), 1);
        assert_eq!(fleet.get(CraftId(1)).unwrap().kind, Kind::Beacon);
        assert!(fleet.get(CraftId(2)).is_none());
        assert!(fleet.apply(CraftId(2), &Event {
            ship: ShipId(2),
            at_t: 0.0,
            change: Change::CutDrive,
        })
        .is_none(), "an event for a craft nobody has heard of is not an error");
        assert_eq!(fleet.remove(CraftId(1)).map(|c| c.kind), Some(Kind::Beacon));
        assert!(fleet.is_empty());
    }

    /// A craft says what it is called, whether or not anyone has named it.
    #[test]
    fn an_unnamed_craft_still_has_a_designation() {
        let mut craft = Craft::at(CraftId(7), Kind::Probe, DVec3::ZERO);
        assert_eq!(craft.designation(), "Probe 7");
        craft.name = Some("Huygens".into());
        assert_eq!(craft.designation(), "Huygens");
    }

    use lc_spacetime::Worldline;

    const HOUR_AGO_US: f64 = -3_600.0 * 1.0e6;

    fn fitted() -> Craft {
        use crate::fitting::{Balance, Loadout};
        let mut craft = Craft::at(CraftId(9), Kind::Ship, DVec3::ZERO);
        craft.fit(Some(Fitting::full(Loadout::STARTING, Balance::DEFAULT, 0.0)));
        craft
    }

    fn cross(craft: &mut Craft, to_ly: DVec3, at_t: f64) -> f64 {
        let event = Event {
            ship: ShipId(9),
            at_t,
            change: Change::Cross { to_ly, drive: craft.rated_drive(at_t) },
        };
        let quoted = craft.cost_of(&event).unwrap();
        craft.apply(&event).unwrap();
        quoted
    }

    #[test]
    fn a_crossing_is_paid_for_when_it_is_ordered_and_spent_as_it_is_flown() {
        let mut craft = fitted();
        let free = craft.free_j_at(0.0);
        let mass = craft.mass_kg_at(0.0);
        let quoted = cross(&mut craft, DVec3::X * 0.05, 0.0);
        assert!(quoted > 0.0 && quoted < free);
        let Motive::Crossing(cruise) = &craft.motion.motive else { panic!("not crossing") };
        let (eta, end) = (cruise.planned_rapidity(), cruise.duration_s());
        assert!((quoted / crate::cost::energy_j(mass, eta, 1.0) - 1.0).abs() < 1.0e-12);
        // Committed at once, so what is free drops before anything is spent.
        assert!((craft.free_j_at(0.0) - (free - quoted)).abs() < 1.0e-9 * free);

        // Half-way, stored has fallen and the commitment with it.
        let fitting = craft.fitting().unwrap();
        let stored_half = fitting.stored_j_at(&craft.motion, end * 0.5);
        assert!(stored_half < free && stored_half > free - quoted);

        // Flown to the end: what was quoted, plus the drain, has gone — a little less, because
        // the account settles every game day and the drain has made the ship lighter for the rest
        // of the burn. That remainder is the drain's mass times the burn's own fraction.
        craft.advance(end + 1.0, end + 1.0);
        assert!(!craft.motion.is_under_way(), "{:?}", craft.motion.motive);
        let living = crate::fitting::Loadout::STARTING.living as f64;
        let drain = living * crate::fitting::Balance::DEFAULT.living_drain_w * (end + 1.0);
        let stored = craft.fitting().unwrap().stored_j_at(&craft.motion, end + 1.0);
        let expected = free - quoted - drain;
        assert!(stored >= expected * (1.0 - 1.0e-12), "{stored} vs {expected}");
        assert!(stored - expected <= drain * quoted / (mass * crate::fitting::C2), "{stored} vs {expected}");
        assert!(craft.mass_kg_at(end + 1.0) < mass);
        // Lighter, so its engines pull harder.
        assert!(craft.rated_drive(end + 1.0).accel_g > 5.0);
    }

    #[test]
    fn cutting_the_drive_returns_what_was_not_flown() {
        let mut flown = fitted();
        let free = flown.free_j_at(0.0);
        let quoted = cross(&mut flown, DVec3::X * 0.05, 0.0);
        let Motive::Crossing(cruise) = &flown.motion.motive else { panic!("not crossing") };
        let cut_at = cruise.duration_s() * 0.25;
        flown
            .apply(&Event { ship: ShipId(9), at_t: cut_at, change: Change::CutDrive })
            .unwrap();
        let left = flown.free_j_at(cut_at);
        assert!(left > free - quoted, "nothing came back: {left}");
        assert!(left < free, "it flew for nothing");
    }

    #[test]
    fn a_burn_spends_the_rapidity_it_changes_by() {
        let mut craft = fitted();
        let mass = craft.mass_kg_at(0.0);
        let before = craft.fitting().unwrap().stored_j_at(&craft.motion, 0.0);
        craft.drift_from(DVec3::ZERO, DVec3::X * 1.0e-4, 0.0);
        let after = craft.fitting().unwrap().stored_j_at(&craft.motion, 0.0);
        let spent = before - after;
        let expected = crate::cost::energy_j(mass, 1.0e-4f64.atanh(), 1.0);
        assert!((spent / expected - 1.0).abs() < 1.0e-9, "{spent} vs {expected}");
    }

    /// An empty fitted ship at rest `au` from the Sun, or on a conic through there with `speed`
    /// of circular.
    fn near_the_sun(system: &Arc<LocalSystem>, au: f64, speed: Option<f64>) -> Craft {
        use crate::fitting::{Account, Balance, Loadout};
        let star = system.star_position_at(0.0).expect("a star");
        let at = star + DVec3::X * au * crate::system::UNIT_M / crate::system::M_PER_LY;
        let mut craft = Craft::at(CraftId(9), Kind::Ship, at);
        let full = Fitting::full(Loadout::STARTING, Balance::DEFAULT, 0.0);
        let empty = Account { stored_j: 0.0, ..full.account() };
        craft.fit(Some(Fitting::from_account(&empty, Balance::DEFAULT)));
        craft.enter(Some(system.clone()), 0.0);
        if let Some(fraction) = speed {
            let mu = crate::star::Star::SOL.mu;
            let v = fraction * (mu / (au * crate::system::UNIT_M)).sqrt();
            craft.drift_from(at, DVec3::Y * v / crate::flight::C_M_S, 0.0);
            craft.apply(&Event { ship: ShipId(9), at_t: 0.0, change: Change::CutDrive }).unwrap();
        }
        craft
    }

    fn stored(craft: &Craft, t: f64) -> f64 {
        craft.fitting().unwrap().stored_j_at(&craft.motion, t)
    }

    /// The anchor, flown rather than computed: a starting ship at rest a tenth of an AU from the
    /// real Sun is about half full after half a year and full after a year.
    #[test]
    fn a_ship_holding_still_near_the_sun_fills_up() {
        let Some(system) = sol() else { return };
        let mut craft = near_the_sun(&system, 0.1, None);
        let capacity = craft.fitting().unwrap().capacity_j_at(0.0);
        let year = crate::flight::JULIAN_YEAR_S;
        let mut t = 0.0;
        while t < 0.5 * year {
            t += 3_600.0 * 6.0;
            craft.advance(t, 3_600.0 * 6.0);
        }
        // The catalogue Sun is not exactly the anchor's 1361 W/m², so a few per cent either way.
        let half = stored(&craft, t) / capacity;
        assert!((half - 0.5).abs() < 0.03, "{half} full after half a year");
        craft.advance(1.1 * year, 0.6 * year);
        assert_eq!(stored(&craft, 1.1 * year), capacity, "it stops at capacity");
    }

    #[test]
    fn nothing_is_collected_under_way_or_between_systems() {
        let Some(system) = sol() else { return };
        let day = crate::solar::SOLAR_STEP_S;
        let mut flying = near_the_sun(&system, 0.1, None);
        let to = flying.motion.position_ly + DVec3::X * 0.01;
        let drive = flying.rated_drive(0.0);
        flying.apply(&Event { ship: ShipId(9), at_t: 0.0, change: Change::Cross { to_ly: to, drive } }).unwrap();
        flying.advance(3.0 * day, 3.0 * day);
        assert!(flying.motion.is_under_way());
        assert_eq!(flying.fitting().unwrap().solar_w(), 0.0);

        let mut outside = near_the_sun(&system, 0.1, None);
        outside.enter(None, 0.0);
        outside.advance(3.0 * day, 3.0 * day);
        assert_eq!(stored(&outside, 3.0 * day), 0.0);
    }

    /// **The midpoint earns its place.** Along an eccentric conic the collected energy is what
    /// midpoint-priced day-long segments give, which is far closer to the true integral than
    /// pricing each segment at its start — and the same whether the craft is stepped by the hour
    /// or by the week.
    #[test]
    fn income_along_an_eccentric_orbit_is_priced_at_each_segments_midpoint() {
        let Some(system) = sol() else { return };
        let day = crate::solar::SOLAR_STEP_S;
        let span = 40.0 * day;
        let mut hourly = near_the_sun(&system, 0.3, Some(0.8));
        assert!(matches!(hourly.motion.motive, Motive::Falling(_)), "{:?}", hourly.motion.motive);
        let mut weekly = hourly.clone();
        let reference = hourly.clone();

        let mut t = 0.0;
        while t < span {
            t = (t + 3_600.0).min(span);
            hourly.advance(t, 3_600.0);
        }
        let mut t = 0.0;
        while t < span {
            t = (t + 7.0 * day).min(span);
            weekly.advance(t, 7.0 * day);
        }
        let (by_hour, by_week) = (stored(&hourly, span), stored(&weekly, span));
        assert!((by_hour / by_week - 1.0).abs() < 1.0e-9, "{by_hour} vs {by_week}");

        let drain = reference.fitting().unwrap().balance.drain_w(&crate::fitting::Loadout::STARTING);
        let power = |t: f64| reference.solar_w_at(t) - drain;
        let (mut midpoint, mut start, mut exact) = (0.0, 0.0, 0.0);
        for k in 0..40 {
            let t0 = k as f64 * day;
            midpoint += power(t0 + 0.5 * day) * day;
            start += power(t0) * day;
            for j in 0..200 {
                exact += power(t0 + (j as f64 + 0.5) * day / 200.0) * day / 200.0;
            }
        }
        assert!((by_hour / midpoint - 1.0).abs() < 1.0e-9, "{by_hour} vs {midpoint}");
        let (mid_err, start_err) = ((midpoint - exact).abs(), (start - exact).abs());
        assert!(mid_err * 10.0 < start_err, "midpoint off by {mid_err}, start by {start_err}");
    }

    /// An idle fitted ship holds its nose perpendicular to the star, and after a flight it swings
    /// back to broadside at its hull's own rate rather than snapping.
    #[test]
    fn an_idle_ship_turns_broadside_to_its_star() {
        let Some(system) = sol() else { return };
        let mut craft = near_the_sun(&system, 0.1, None);
        let to_star = |craft: &Craft, t: f64| {
            (system.star_position_at(t).unwrap() - craft.motion.position_ly).normalize()
        };
        // Idle since before anything: already round, with no turn left to make.
        assert!(craft.facing_at(0.0).unwrap().dot(to_star(&craft, 0.0)).abs() < 1.0e-9);

        // Nothing unfitted turns: a probe keeps the attitude it was left with.
        let mut probe = Craft::at(CraftId(3), Kind::Probe, craft.motion.position_ly);
        probe.enter(Some(system.clone()), 0.0);
        assert_eq!(probe.facing_at(1.0e4), Some(DVec3::X));

        // Sent somewhere, it points where the plan needs it.
        let drive = craft.rated_drive(0.0);
        // Straight out from the star, so the plan's nose is along it and not broadside.
        let to = craft.motion.position_ly + DVec3::X * 0.05;
        craft.apply(&Event { ship: ShipId(9), at_t: 0.0, change: Change::Cross { to_ly: to, drive } }).unwrap();
        let flying = craft.facing_at(600.0).unwrap();
        assert!(flying.dot(to_star(&craft, 600.0)).abs() > 0.5, "{flying} is still broadside");

        // Drive cut: the turn back starts from the nose the crossing left, and takes a quarter
        // turn at the hull's rate to arrive.
        let cut_at = 600.0;
        craft.apply(&Event { ship: ShipId(9), at_t: cut_at, change: Change::CutDrive }).unwrap();
        assert!((craft.facing_at(cut_at).unwrap() - flying).length() < 1.0e-9, "the nose snapped");
        let quarter = 0.5 * std::f64::consts::PI / craft.slew_rate_rad_s();
        let part_way = craft.facing_at(cut_at + 0.25 * quarter).unwrap();
        assert!(part_way.dot(flying) < 0.999, "it did not begin turning");
        assert!(part_way.dot(to_star(&craft, cut_at)).abs() > 1.0e-6, "it arrived at once");
        let settled = craft.facing_at(cut_at + quarter * 1.01).unwrap();
        assert!(settled.dot(to_star(&craft, cut_at + quarter)).abs() < 1.0e-6, "{settled}");
        assert!(settled.is_normalized());
    }

    /// **A plan is planned from the attitude the craft then records.** They are two halves of one
    /// hand-over, and a plan is re-planned from the recorded one at the far end of a wire or a
    /// checkpoint — so a plan made from anything else comes back as a different flight. This went
    /// wrong the moment an idle ship's nose could move on its own.
    #[test]
    fn a_plan_starts_from_the_attitude_the_craft_records() {
        let Some(system) = sol() else { return };
        let mut craft = near_the_sun(&system, 0.5, None);
        // Broadside, so the nose is not the attitude the last order left it at.
        let nose = craft.facing_at(0.0).unwrap();
        assert!((nose - craft.motion.attitude).length() > 0.1, "premise: the nose has moved");

        let drive = craft.rated_drive(0.0);
        let to = craft.motion.position_ly + DVec3::X * 0.05;
        craft.apply(&Event { ship: ShipId(9), at_t: 0.0, change: Change::Cross { to_ly: to, drive } }).unwrap();
        let Motive::Crossing(cruise) = &craft.motion.motive else { panic!("not crossing") };
        assert_eq!(cruise.initial_attitude(), craft.motion.attitude);
        assert_eq!(craft.motion.attitude, nose, "it planned from somewhere the nose had not been");
    }

    #[test]
    fn a_broadside_nose_is_the_nearest_perpendicular() {
        let s = DVec3::X;
        assert_eq!(broadside_nose(DVec3::new(1.0, 1.0, 0.0), s), DVec3::Y);
        assert_eq!(broadside_nose(DVec3::Z, s), DVec3::Z);
        let head_on = broadside_nose(-DVec3::X, s);
        assert!(head_on.dot(s).abs() < 1.0e-12 && head_on.is_normalized(), "{head_on}");
    }

    #[test]
    fn a_refit_that_grows_the_hull_lengthens_it() {
        use crate::fitting::Loadout;
        let mut craft = fitted();
        assert!((craft.length_m - 500.0).abs() < 1.0e-9);
        craft.begin_refit(Loadout { slots: 22, ..Loadout::STARTING }, 0.0).unwrap();
        assert!(craft.is_refitting(1.0));
        let year = crate::flight::JULIAN_YEAR_S;
        craft.advance(year, year);
        assert!(!craft.is_refitting(year));
        assert_eq!(craft.fitting().unwrap().loadout.slots, 22);
        assert!((craft.length_m / (500.0 * 1.1f64.cbrt()) - 1.0).abs() < 1.0e-12);
    }

    fn drifting(beta: DVec3, since_s: f64) -> Craft {
        let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        craft.motion.beta = beta;
        craft.motion.set_adrift(since_s);
        craft
    }

    /// **The bug this history exists to kill.**
    ///
    /// A motive is a closed form total in `t`, so the *current* one answers about times before
    /// it was ever flown — and `Drifting` extrapolates backwards, so a burn would move the ship
    /// in the past and change how fast it was going there. Every retarded solve reads that, so
    /// an observer a light-hour away would see a maneuvere the instant it happened.
    #[test]
    fn a_burn_does_not_rewrite_where_the_ship_was_an_hour_ago() {
        let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        let (was_at, was_going) = {
            let line = craft.worldline();
            (line.position_at(HOUR_AGO_US), line.velocity_at(HOUR_AGO_US))
        };
        assert_eq!(was_going, DVec3::ZERO, "premise: it was sitting still");

        // It lights the drive, now.
        craft.apply(&Event {
            ship: motion::ShipId(1),
            at_t: 0.0,
            change: crate::motion::Change::Cross {
                to_ly: DVec3::X,
                drive: crate::flight::Drive::DEFAULT,
            },
        })
        .expect("a crossing");
        assert!(craft.motion.is_under_way(), "premise: it actually burned");

        let line = craft.worldline();
        assert_eq!(line.velocity_at(HOUR_AGO_US), was_going, "the burn reached back an hour");
        assert_eq!(line.position_at(HOUR_AGO_US), was_at, "and moved it there too");
    }

    /// The same, for the motive that gets it worst: a drift read before it began runs the new
    /// velocity backwards from the point the burn happened at.
    #[test]
    fn a_change_of_drift_does_not_reach_back_either() {
        let mut craft = drifting(DVec3::ZERO, 0.0);
        let before = craft.worldline().position_at(HOUR_AGO_US);

        craft.remembering(0.0, |craft| {
            craft.motion.beta = DVec3::new(0.0, 1.0e-3, 0.0);
            craft.motion.set_adrift(0.0);
        });
        assert_eq!(craft.worldline().position_at(HOUR_AGO_US), before);
        assert_eq!(craft.worldline().velocity_at(HOUR_AGO_US), DVec3::ZERO);
        // And the present is the new motion, or nothing has happened at all.
        assert_ne!(craft.worldline().velocity_at(1.0e6), DVec3::ZERO);
    }

    /// Doing the same thing for a long time costs nothing: a stretch is recorded when the
    /// motive *changes*, not when it is looked at.
    #[test]
    fn a_craft_that_keeps_doing_one_thing_remembers_one_thing() {
        let mut craft = drifting(DVec3::new(1.0e-6, 0.0, 0.0), 0.0);
        for k in 1..500 {
            craft.advance(k as f64 * 10.0, 10.0);
        }
        assert_eq!(craft.remembered(), 0, "a steady drift recorded {} stretches", craft.remembered());
    }

    /// The memory is bounded, and running off the end is answered by refusing rather than by
    /// guessing — which is what [`Flight::defined_over`] tells the solver.
    #[test]
    fn a_craft_that_maneuveres_forever_forgets_its_oldest_stretches() {
        let mut craft = drifting(DVec3::ZERO, 0.0);
        for k in 1..=(HISTORY_STRETCHES + 40) {
            let at = k as f64;
            craft.remembering(at, |craft| {
                craft.motion.beta = DVec3::X * (k as f64 * 1.0e-9);
                craft.motion.set_adrift(at);
            });
        }
        assert_eq!(craft.remembered(), HISTORY_STRETCHES, "the history is not bounded");
        assert!(craft.known_from_s() > 0.0, "it forgot without saying how far back it can go");

        // The solver's contract: a root before that is no root at all.
        let (from, to) = craft.worldline().defined_over();
        assert_eq!(from, craft.known_from_s() * 1.0e6);
        assert!(to.is_infinite());
    }

    /// A stretch is stamped with when it *ended*, so the state in force is the one whose end
    /// has not been reached.
    #[test]
    fn the_stretch_in_force_is_the_one_that_had_not_ended_yet() {
        let mut craft = drifting(DVec3::ZERO, 0.0);
        let steps = [(100.0, 1.0e-6), (200.0, 2.0e-6), (300.0, 3.0e-6)];
        for (at, beta) in steps {
            craft.remembering(at, |craft| {
                craft.motion.beta = DVec3::X * beta;
                craft.motion.set_adrift(at);
            });
        }
        let at = |s: f64| craft.worldline().velocity_at(s * 1.0e6).x;
        assert_eq!(at(50.0), 0.0, "before the first change it was still at rest");
        assert_eq!(at(150.0), 1.0e-6);
        assert_eq!(at(250.0), 2.0e-6);
        assert_eq!(at(350.0), 3.0e-6, "past the last change it is the current motive");
    }

    /// Mass follows size, and size is cubic — which is the fact to have in mind before being
    /// surprised by what the big end of the range weighs.
    #[test]
    fn mass_follows_the_cube_of_the_length() {
        let mut small = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        small.length_m = LENGTH_RANGE_M.0;
        let mut large = Craft::at(CraftId(2), Kind::Ship, DVec3::ZERO);
        large.length_m = LENGTH_RANGE_M.1;

        let ratio = large.mass_kg() / small.mass_kg();
        let lengths = LENGTH_RANGE_M.1 / LENGTH_RANGE_M.0;
        assert!(
            (ratio - lengths.powi(3)).abs() < ratio * 1.0e-9,
            "{ratio} against {}",
            lengths.powi(3),
        );
        // And the small end is a number a person can hold: a few million tonnes.
        assert!(small.mass_kg() > 1.0e9 && small.mass_kg() < 1.0e10, "{}", small.mass_kg());
    }

    /// The ovoid's own volume, not a box or a sphere: five long by three across by one deep.
    #[test]
    fn volume_is_the_ovoid_the_hull_is_drawn_as() {
        let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        craft.length_m = 1_000.0;
        let half = 500.0;
        let want = 4.0 / 3.0
            * std::f64::consts::PI
            * half
            * (half * BEAM_PER_LENGTH)
            * (half * HEIGHT_PER_LENGTH);
        assert!((craft.volume_m3() - want).abs() < want * 1.0e-12);
        // A box of the same extents would be a long way out, which is what makes this worth
        // checking rather than assuming.
        let box_volume = 1_000.0 * 1_000.0 * BEAM_PER_LENGTH * 1_000.0 * HEIGHT_PER_LENGTH;
        assert!(craft.volume_m3() < box_volume * 0.6);
    }

    /// Two craft of a size are not two craft of a mass: what fills a hull is what it is for.
    #[test]
    fn what_a_hull_is_for_changes_what_it_weighs() {
        let at = |kind| {
            let mut craft = Craft::at(CraftId(1), kind, DVec3::ZERO);
            craft.length_m = 1_000.0;
            craft.mass_kg()
        };
        assert!(at(Kind::Probe) > at(Kind::Ship));
        assert!(at(Kind::Ship) > at(Kind::Relay));
        assert!(at(Kind::Relay) > at(Kind::Beacon));
    }


    /// What a burn costs in light, and the shape of the dependence: everything about it scales
    /// with what is being pushed and how hard.
    #[test]
    fn a_burn_radiates_with_the_mass_and_the_acceleration() {
        use crate::flight::Drive;
        let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        craft.length_m = 500.0;
        let at = |accel_g: f64| craft.motion.drive.jet_power_w(craft.mass_kg(), accel_g);

        // Linear in both, because the thrust is and the exhaust speed is fixed.
        assert!((at(10.0) / at(5.0) - 2.0).abs() < 1.0e-9);
        let mut bigger = craft.clone();
        bigger.length_m = 1_000.0;
        let ratio = bigger.motion.drive.jet_power_w(bigger.mass_kg(), 5.0) / at(5.0);
        assert!((ratio - 8.0).abs() < 1.0e-9, "twice the ship is eight times the mass: {ratio}");

        // And the number itself is the one worth having seen: a fair fraction of a star.
        let full = at(Drive::DEFAULT.accel_g);
        assert!(full > 1.0e17 && full < 1.0e19, "{full} W");
    }

    /// Nothing is lit unless something is thrusting, and a ballistic arc is not thrusting
    /// however hard it is falling.
    #[test]
    fn a_coasting_ship_puts_nothing_out() {
        let craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        assert_eq!(craft.jet_power_w(0.0), 0.0, "a drifting ship has its engine off");

        let mut under_way = craft.clone();
        under_way.motion.begin_crossing(
            crate::flight::Cruise::plan(DVec3::ZERO, DVec3::X, 0.0, crate::flight::Drive::DEFAULT),
            None,
        );
        assert!(under_way.jet_power_w(1.0) > 0.0, "a ship on a crossing is burning");
    }

    /// A station-keeping thruster is not a torch, so a beacon correcting itself is not the
    /// same event as a ship getting under way.
    #[test]
    fn a_beacon_is_far_quieter_than_a_ship() {
        let ship = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        let mut beacon = Craft::at(CraftId(2), Kind::Beacon, DVec3::ZERO);
        // The same size, so only what it is for is different.
        beacon.length_m = ship.length_m;
        let power = |c: &Craft| c.motion.drive.jet_power_w(c.mass_kg(), c.motion.drive.accel_g);
        assert!(power(&beacon) < power(&ship) / 100.0, "{} against {}", power(&beacon), power(&ship));
    }

}
