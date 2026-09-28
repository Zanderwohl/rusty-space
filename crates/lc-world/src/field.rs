//! The field's heat account. See `lightcone/docs/30-the-field.md`.
//!
//! `Q` is the heat the field holds, J. What it radiates goes as `T⁴` and so in proportion to `Q`,
//! which makes the account linear, `dQ/dt = P − Q/τ`, and gives every segment of constant inputs a
//! closed form in both directions. Nothing here reads a form or a craft: envelope area, capacity and
//! inputs are arguments, so settlement, collapse scheduling and the editor's preview share it.
//!
//! Powers are W, energies J, times s, areas m².

use crate::fitting::Balance;

/// What the field does with what arrives at it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Clear,
    Black,
}

impl Mode {
    /// Of what arrives, the fraction absorbed. The rest is reflected and does nothing to the ship.
    pub fn absorptivity(self, clear_absorptivity: f64) -> f64 {
        match self {
            Mode::Clear => clear_absorptivity,
            Mode::Black => 1.0,
        }
    }
}

/// One ship's field: its envelope and the balance's constants for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    pub area_m2: f64,
    /// J/m² at collapse.
    pub capacity_j_m2: f64,
    /// Must be positive and finite.
    pub tau_s: f64,
    /// The temperature at [`idle_j_m2`](Self::idle_j_m2).
    pub idle_k: f64,
    /// J/m².
    pub idle_j_m2: f64,
}

impl Field {
    /// The field on an envelope of `area_m2`, with the balance's constants.
    pub fn of(area_m2: f64, balance: &Balance) -> Field {
        Field {
            area_m2,
            capacity_j_m2: balance.field_capacity,
            tau_s: balance.field_tau_s,
            idle_k: balance.field_idle_k,
            idle_j_m2: balance.field_idle_j_m2(),
        }
    }

    /// `Q_max`: the heat at which the field collapses.
    pub fn heat_max_j(&self) -> f64 {
        self.capacity_j_m2 * self.area_m2
    }

    /// `T = T_idle (q / q_idle)^¼`, from the energy of trapped radiation going as `T⁴`.
    ///
    /// Zero for no heat; infinite for heat on a field with no area or no idle heat, rather than NaN.
    pub fn temperature_k(&self, heat_j: f64) -> f64 {
        if heat_j <= 0.0 {
            return 0.0;
        }
        let idle_j = self.idle_j_m2 * self.area_m2;
        if idle_j <= 0.0 {
            return f64::INFINITY;
        }
        self.idle_k * (heat_j / idle_j).sqrt().sqrt()
    }

    /// Where heat tends under a constant net `power_w`: `P τ`.
    pub fn equilibrium_j(&self, power_w: f64) -> f64 {
        power_w * self.tau_s
    }

    /// The sustained power that would bring the field to `Q_max` in the limit.
    pub fn rated_load_w(&self) -> f64 {
        self.heat_max_j() / self.tau_s
    }

    /// What a collapse releases as light: `Q_max` and everything stored, committed energy included.
    /// The parts' own mass is not released.
    pub fn released_j(&self, stored_j: f64) -> f64 {
        self.heat_max_j() + stored_j.max(0.0)
    }

    /// `Q(t) = P τ + (Q₀ − P τ) e^(−t/τ)`, over `dt_s` of constant net `power_w`.
    pub fn heat_after_j(&self, heat_j: f64, power_w: f64, dt_s: f64) -> f64 {
        debug_assert!(self.tau_s > 0.0 && self.tau_s.is_finite());
        // Written as Q₀ plus a fraction of the gap, with expm1, so a short step returns Q₀ plus
        // about P·dt rather than the difference of two numbers near P τ.
        let reached = -(-dt_s / self.tau_s).exp_m1();
        heat_j + (self.equilibrium_j(power_w) - heat_j) * reached
    }

    /// Time until heat rises to `threshold_j` under constant `power_w`: zero if it is already
    /// there, `None` if it never gets there because the equilibrium is at or below it.
    pub fn time_to_rise_s(&self, heat_j: f64, threshold_j: f64, power_w: f64) -> Option<f64> {
        if heat_j >= threshold_j {
            return Some(0.0);
        }
        self.time_between(threshold_j - heat_j, self.equilibrium_j(power_w) - threshold_j)
    }

    /// Time until heat falls to `threshold_j`; the mirror of [`time_to_rise_s`](Self::time_to_rise_s).
    pub fn time_to_fall_s(&self, heat_j: f64, threshold_j: f64, power_w: f64) -> Option<f64> {
        if heat_j <= threshold_j {
            return Some(0.0);
        }
        self.time_between(heat_j - threshold_j, threshold_j - self.equilibrium_j(power_w))
    }

    /// `τ ln((P τ − Q₀) / (P τ − Q_threshold))` as `τ ln(1 + gap / beyond)`: both positive when the
    /// threshold is reachable, and `ln_1p` keeps a threshold just ahead of `Q₀` from canceling.
    /// An equilibrium on the threshold is an asymptote, and `beyond` too small to divide by is the
    /// same asymptote, so both are `None`.
    fn time_between(&self, gap_j: f64, beyond_j: f64) -> Option<f64> {
        if beyond_j <= 0.0 {
            return None;
        }
        let t = self.tau_s * (gap_j / beyond_j).ln_1p();
        t.is_finite().then_some(t)
    }

    /// Net heat power while storage has room.
    pub fn heat_filling_w(&self, segment: &Segment) -> f64 {
        segment.absorbed_w() - segment.stored_w() + segment.internal_w
    }

    /// Net heat power once storage is full and held there: conversion stores only what the draw
    /// takes out, and the rest of what is absorbed is heat.
    pub fn heat_full_w(&self, segment: &Segment) -> f64 {
        segment.absorbed_w() - segment.draw_w + segment.internal_w
    }

    /// Settles `dt_s` of `segment` from `heat_j`, splitting it where storage fills and where heat
    /// reaches the floor.
    pub fn settle(&self, segment: &Segment, heat_j: f64, dt_s: f64) -> Settled {
        let mut settled = Settled { heat_j, storage_j: 0.0, filled_s: None, from_heat_j: 0.0 };
        let mut at_s = 0.0;
        for stretch in self.stretches(segment, heat_j, dt_s) {
            if stretch.full {
                settled.filled_s.get_or_insert(at_s);
            }
            settled.heat_j = self.heat_after_j(stretch.heat_j, stretch.heat_w, stretch.dt_s);
            settled.storage_j += stretch.storage_w * stretch.dt_s;
            settled.from_heat_j += stretch.from_heat_w * stretch.dt_s;
            at_s += stretch.dt_s;
        }
        settled
    }

    /// When heat first reaches `threshold_j` from below under `segment` left running, fill and
    /// floor split included. What a collapse is scheduled from.
    pub fn segment_time_to_rise_s(&self, segment: &Segment, heat_j: f64, threshold_j: f64) -> Option<f64> {
        self.segment_time_to(segment, heat_j, |field, heat, power| {
            field.time_to_rise_s(heat, threshold_j, power)
        })
    }

    /// When heat first falls to `threshold_j` under `segment` left running, fill and floor split
    /// included.
    pub fn segment_time_to_fall_s(&self, segment: &Segment, heat_j: f64, threshold_j: f64) -> Option<f64> {
        self.segment_time_to(segment, heat_j, |field, heat, power| {
            field.time_to_fall_s(heat, threshold_j, power)
        })
    }

    fn segment_time_to(
        &self,
        segment: &Segment,
        heat_j: f64,
        time_to: impl Fn(&Self, f64, f64) -> Option<f64>,
    ) -> Option<f64> {
        let mut at_s = 0.0;
        for stretch in self.stretches(segment, heat_j, f64::INFINITY) {
            if let Some(t) = time_to(self, stretch.heat_j, stretch.heat_w).filter(|&t| t <= stretch.dt_s) {
                return Some(at_s + t);
            }
            at_s += stretch.dt_s;
        }
        None
    }

    /// Cut where storage fills and where heat reaches or leaves the floor. At the floor the heat
    /// made goes out with the emission as it is made, so storage pays the emission less that.
    pub(crate) fn stretches(&self, segment: &Segment, heat_j: f64, dt_s: f64) -> Vec<Stretch> {
        let emitted_w = segment.emitted_w.max(0.0);
        let filling_w = segment.stored_w() - segment.draw_w;
        let floor_w = self.heat_filling_w(segment) + filling_w - emitted_w;
        let (mut heat_j, mut room_j, mut at_s) = (heat_j.max(0.0), segment.room_j, 0.0);
        let mut stretches = Vec::with_capacity(3);
        while at_s < dt_s {
            let full = filling_w > 0.0 && room_j <= 0.0;
            let made_w = if full { self.heat_full_w(segment) } else { self.heat_filling_w(segment) };
            let at_floor = emitted_w > 0.0 && heat_j <= 0.0 && made_w <= emitted_w;
            let (heat_w, storage_w, from_heat_w) = match (at_floor, full) {
                (true, _) => (0.0, floor_w, made_w.max(0.0)),
                (false, true) => (made_w - emitted_w, 0.0, emitted_w),
                (false, false) => (made_w - emitted_w, filling_w, emitted_w),
            };
            let fills_s = if !full && storage_w > 0.0 { room_j / storage_w } else { f64::INFINITY };
            let floors_s = match heat_w < 0.0 && !at_floor {
                true => self.time_to_fall_s(heat_j, 0.0, heat_w).unwrap_or(f64::INFINITY),
                false => f64::INFINITY,
            };
            // A third stretch is never left: full and rising, or at the floor and draining.
            let len_s = if stretches.len() == 2 { dt_s - at_s } else { (dt_s - at_s).min(fills_s).min(floors_s) };
            stretches.push(Stretch { dt_s: len_s, heat_j, heat_w, storage_w, from_heat_w, full: full && !at_floor });
            at_s += len_s;
            if at_s >= dt_s {
                break;
            }
            heat_j = if len_s == floors_s { 0.0 } else { self.heat_after_j(heat_j, heat_w, len_s) };
            room_j = if len_s == fills_s { 0.0 } else { room_j - storage_w * len_s };
        }
        stretches
    }
}

/// Part of a segment over which heat's power and storage's rate are both constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stretch {
    pub dt_s: f64,
    /// At the stretch's start.
    pub heat_j: f64,
    /// Net, the emission included. Zero at the floor.
    pub heat_w: f64,
    /// Net, the emission included.
    pub storage_w: f64,
    /// Of [`Segment::emitted_w`], what heat supplies.
    pub from_heat_w: f64,
    pub full: bool,
}

/// Constant inputs to the field, as far as the next change of any of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// Arriving at the field from outside, before absorptivity: starlight, beams, a neighbor's glow.
    pub arriving_w: f64,
    /// [`Mode::absorptivity`].
    pub absorptivity: f64,
    /// Made inside the ship, and all of it heat: the living drain, what dismantling loses. Not
    /// conversion's loss, which the segment works out itself, nor the drive's waste below ε = 1,
    /// which [`emitted_w`](Self::emitted_w) cannot draw and so is kept out of the segment.
    pub internal_w: f64,
    /// The most absorbed power conversion can take in.
    pub rating_w: f64,
    /// Of what is converted, the fraction stored.
    pub efficiency: f64,
    /// What storage can still take at the segment's start. Zero or less is full.
    pub room_j: f64,
    /// Drawn out of storage meanwhile: it delays filling, and once full it is all conversion stores.
    /// Heat from the same draw is the caller's to put in `internal_w`.
    pub draw_w: f64,
    /// A drive's exhaust or anything else the ship lights, drawn from heat while the field holds any
    /// and from storage for the rest.
    pub emitted_w: f64,
}

impl Segment {
    pub fn absorbed_w(&self) -> f64 {
        self.arriving_w * self.absorptivity
    }

    /// Of what is absorbed, what conversion takes in while storage has room.
    pub fn converted_w(&self) -> f64 {
        self.absorbed_w().min(self.rating_w).max(0.0)
    }

    /// Into storage while it has room.
    pub fn stored_w(&self) -> f64 {
        self.efficiency * self.converted_w()
    }

    /// When storage fills, from the segment's start, after which it is held full: zero if it starts
    /// full, `None` if the draw keeps up with conversion, which leaves storage room all segment long
    /// however full it started.
    pub fn fill_s(&self) -> Option<f64> {
        let net_w = self.stored_w() - self.draw_w;
        if net_w <= 0.0 {
            return None;
        }
        if self.room_j <= 0.0 {
            return Some(0.0);
        }
        let t = self.room_j / net_w;
        t.is_finite().then_some(t)
    }
}

/// A burst jumps `Q` at the instant it arrives. It comes faster than any rating, so all of it is
/// heat: empty storage takes in sustained power, never a burst.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Burst {
    /// J arriving from outside — a beam's pulse, a collapse's spike — before absorptivity.
    Arriving(f64),
    /// J of storage vented for want of room. Already inside, so no absorptivity applies.
    Vent(f64),
}

impl Burst {
    pub fn heat_j(self, absorptivity: f64) -> f64 {
        match self {
            Burst::Arriving(j) => j * absorptivity,
            Burst::Vent(j) => j,
        }
    }
}

/// The most of what surrounds it a receiver takes, from one source or all of them together: the
/// half a receiver in contact faces.
pub const CONTACT_FRACTION: f64 = 0.5;

/// Of what a source radiates isotropically, the fraction a shadow of `shadow_m2` at `distance_m`
/// takes: `A / (4π d²)`. At most [`CONTACT_FRACTION`], where the inverse square would give more, or
/// infinity for ships stacked at one point.
pub fn received_fraction(shadow_m2: f64, distance_m: f64) -> f64 {
    if shadow_m2 <= 0.0 {
        return 0.0;
    }
    let sphere_m2 = 4.0 * std::f64::consts::PI * distance_m * distance_m;
    (shadow_m2 / sphere_m2).min(CONTACT_FRACTION)
}

/// Inside this distance a spike of `spike_j` takes a field with `headroom_j` left to collapse:
/// `√(α E A / (4π H))`. Infinite for no headroom. See 30 §Proximity.
pub fn lethal_radius_m(absorptivity: f64, spike_j: f64, shadow_m2: f64, headroom_j: f64) -> f64 {
    (absorptivity * spike_j * shadow_m2 / (4.0 * std::f64::consts::PI * headroom_j.max(0.0))).sqrt()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settled {
    pub heat_j: f64,
    /// Net change in storage: conversion less the draw and its part of the emission. Negative when
    /// the draw outruns conversion, and emptying storage is the caller's to handle.
    pub storage_j: f64,
    /// When storage filled, if it did within the step.
    pub filled_s: Option<f64>,
    /// Of what was emitted, what heat supplied.
    pub from_heat_j: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fitting::{FIELD_ANCHOR_ME, RATED_LOAD_AU, STARTING_ENVELOPE_M2};
    use crate::form::capacity::Capacities;
    use crate::form::grid::FormGrid;
    use crate::form::Form;
    use crate::solar::{self, SOLAR_CONSTANT_W_M2};
    use crate::system::UNIT_M as AU_M;

    const AREA_M2: f64 = 4.0e5;

    /// The starting form's field, capacities and broadside, and the starlight it takes in.
    struct Start {
        field: Field,
        caps: Capacities,
        broadside_m2: f64,
        /// On the star's output.
        gain: f64,
    }

    impl Start {
        fn new(b: &Balance) -> Start {
            let form = Form::starting();
            let grid = FormGrid::new(&form, b).unwrap();
            let caps = Capacities::of(&form, b);
            Start { field: Field::of(grid.envelope_area_m2(), b), caps, broadside_m2: grid.broadside_m2(), gain: b.solar_gain }
        }

        /// Broadside, `d_au` from a Sun-like star, before absorptivity.
        fn starlight_w(&self, d_au: f64) -> f64 {
            self.gain * flux_w_m2(d_au) * self.broadside_m2
        }

        /// Black, with storage `room_j` short of full.
        fn segment(&self, b: &Balance, d_au: f64, room_j: f64) -> Segment {
            Segment {
                arriving_w: self.starlight_w(d_au),
                absorptivity: Mode::Black.absorptivity(b.clear_absorptivity),
                internal_w: self.caps.drain_w,
                rating_w: self.caps.aperture_w,
                efficiency: b.conversion_efficiency,
                room_j,
                draw_w: self.caps.drain_w,
                emitted_w: 0.0,
            }
        }
    }

    fn flux_w_m2(d_au: f64) -> f64 {
        SOLAR_CONSTANT_W_M2 / (d_au * d_au)
    }

    fn me(b: &Balance) -> f64 {
        b.module_energy_j()
    }

    #[test]
    fn the_idle_starting_ship_sits_at_field_idle_k() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let idle = equilibrium_k(&start.field, start.caps.drain_w);
        assert!(close(idle, b.field_idle_k, 1e-6), "{idle}");
    }

    /// Capacity is a lever a shard may turn; the idle anchor must not move with it.
    #[test]
    fn the_idle_anchor_does_not_ride_on_capacity() {
        let b = Balance { field_capacity: 2.0 * Balance::DEFAULT.field_capacity, ..Balance::DEFAULT };
        let start = Start::new(&b);
        let idle = equilibrium_k(&start.field, start.caps.drain_w);
        assert!(close(idle, b.field_idle_k, 1e-6), "{idle}");
    }

    #[test]
    fn the_starting_envelope_is_pinned() {
        let area_m2 = Start::new(&Balance::DEFAULT).field.area_m2;
        assert!(close(STARTING_ENVELOPE_M2, area_m2, 1e-6), "pinned {STARTING_ENVELOPE_M2}, the starting form solves to {area_m2:?}");
    }

    #[test]
    fn field_capacity_is_anchored_on_the_starting_envelope() {
        let b = Balance::DEFAULT;
        let heat_max_j = Start::new(&b).field.heat_max_j();
        assert!(close(heat_max_j, FIELD_ANCHOR_ME * me(&b), 1e-6), "{heat_max_j}");
    }

    #[test]
    fn field_tau_is_anchored_on_a_full_starting_ship_at_rated_load() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let field = start.field;
        let full = start.segment(&b, RATED_LOAD_AU, 0.0);
        let heat_w = field.heat_full_w(&full);
        let solved = field.heat_max_j() / heat_w;
        assert!(close(b.field_tau_s, solved, 1e-6), "DEFAULT has {}, the starting form solves to {solved:?}", b.field_tau_s);
        assert!(close(field.equilibrium_j(heat_w), field.heat_max_j(), 1e-6));
        let closer = start.segment(&b, RATED_LOAD_AU * (1.0 - 1e-5), 0.0);
        assert!(field.time_to_rise_s(0.0, field.heat_max_j(), field.heat_full_w(&closer)).is_some());
        let farther = start.segment(&b, RATED_LOAD_AU * (1.0 + 1e-5), 0.0);
        assert_eq!(field.time_to_rise_s(0.0, field.heat_max_j(), field.heat_full_w(&farther)), None);
    }

    /// `τ` is solved on the broadside the gain is solved on.
    #[test]
    fn the_starlight_anchored_on_is_what_arrives() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let luminosity_w = SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * AU_M * AU_M;
        let today_w = solar::intake_w(&b, start.broadside_m2, luminosity_w, RATED_LOAD_AU * AU_M);
        assert!(close(start.starlight_w(RATED_LOAD_AU), today_w, 1e-9), "{} {today_w}", start.starlight_w(RATED_LOAD_AU));
    }

    fn equilibrium_k(field: &Field, power_w: f64) -> f64 {
        field.temperature_k(field.equilibrium_j(power_w))
    }

    fn close(got: f64, want: f64, rel: f64) -> bool {
        (got - want).abs() <= rel * want.abs()
    }

    #[test]
    fn the_worked_numbers_are_thirtys() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let field = start.field;
        let me = me(&b);
        assert!(close(field.tau_s, 1.84e6, 1e-3), "τ {}", field.tau_s);
        assert!(close(field.rated_load_w(), 7.6e19, 1e-2), "rated {}", field.rated_load_w());
        assert!(close(start.caps.aperture_w, 1.1e20, 3e-2));
        let at_collapse = field.temperature_k(field.heat_max_j());
        assert!((at_collapse - 4_577.0).abs() < 0.5, "{at_collapse}");
        let peak_nm = 2.897_771_955e-3 / at_collapse * 1e9;
        assert!((peak_nm - 630.0).abs() < 5.0, "{peak_nm}");
        assert!((field.temperature_k(start.caps.drain_w * field.tau_s) - 400.0).abs() < 1e-9);

        for (d_au, filling_k, full_k) in
            [(5.0, 444.0, 458.0), (1.0, 772.0, 1_024.0), (0.1, 2_396.0, 3_237.0), (0.05, 3_388.0, 4_577.0)]
        {
            let segment = start.segment(&b, d_au, 30.0 * me);
            assert!(segment.stored_w() > segment.draw_w, "{d_au} AU cannot hold storage full");
            let filling = equilibrium_k(&field, field.heat_filling_w(&segment));
            let full = equilibrium_k(&field, field.heat_full_w(&segment));
            assert!((filling - filling_k).abs() < 0.5, "{d_au} AU filling {filling}");
            assert!((full - full_k).abs() < 0.5, "{d_au} AU full {full}");
        }

        let idle_j = start.caps.drain_w * field.tau_s;
        let vented = field.temperature_k(idle_j + Burst::Vent(5.0 * me).heat_j(1.0));
        assert!((vented - 3_850.0).abs() < 5.0, "{vented}");
        let clear_above = field.temperature_k(b.auto_clear_above * field.heat_max_j());
        let black_below = field.temperature_k(b.auto_black_below * field.heat_max_j());
        assert!((clear_above - 3_850.0).abs() < 5.0, "{clear_above}");
        assert!((black_below - 3_400.0).abs() < 15.0, "{black_below}");

        let filling_j = field.equilibrium_j(field.heat_filling_w(&start.segment(&b, 0.1, 30.0 * me)));
        assert!(filling_j + 5.0 * me < field.heat_max_j());
        assert!(filling_j + 10.0 * me > field.heat_max_j());

        // A full Clear ship at rated load: absorbed starlight goes as 1/d².
        let clear = b.clear_absorptivity * start.starlight_w(0.05);
        let d_au = 0.05 * (clear / field.rated_load_w()).sqrt();
        assert!((d_au - 0.027).abs() < 5e-4, "{d_au}");
    }

    /// 30's square–cube table: ships scaled from 500 m with a tenth of their slots living.
    #[test]
    fn bigger_ships_idle_hotter_and_dive_the_same() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        for (length_m, idle_k, full_k) in [(500.0, 476.0, 3_237.0), (5_000.0, 846.0, 3_237.0), (50_000.0, 1_504.0, 3_237.0)] {
            let scale = length_m / 500.0;
            let field = Field { area_m2: start.field.area_m2 * scale * scale, ..start.field };
            let living_w = 2.0 * scale.powi(3) * start.caps.drain_w;
            let segment = Segment {
                arriving_w: start.starlight_w(0.1) * scale * scale,
                internal_w: living_w,
                draw_w: living_w,
                ..start.segment(&b, 0.1, 0.0)
            };
            let idle = equilibrium_k(&field, living_w);
            let full = equilibrium_k(&field, field.heat_full_w(&segment));
            assert!((idle - idle_k).abs() < 0.5, "{length_m} m idle {idle}");
            assert!((full - full_k).abs() < 0.5, "{length_m} m full {full}");
        }
    }

    fn test_field() -> Field {
        Field { area_m2: AREA_M2, capacity_j_m2: 1.0e21, tau_s: 1.84e6, idle_k: 400.0, idle_j_m2: 2.0e16 }
    }

    #[test]
    fn temperature_goes_as_the_fourth_root_of_heat() {
        let field = test_field();
        let idle_j = field.idle_j_m2 * field.area_m2;
        assert_eq!(field.temperature_k(idle_j), 400.0);
        assert!(close(field.temperature_k(16.0 * idle_j), 800.0, 1e-12));
        assert!(close(field.temperature_k(81.0 * idle_j), 1_200.0, 1e-12));
        // Twice the area at twice the heat is the same field, per square meter.
        let doubled = Field { area_m2: 2.0 * field.area_m2, ..field };
        assert!(close(doubled.temperature_k(2.0 * idle_j), 400.0, 1e-12));
        assert_eq!(field.temperature_k(0.0), 0.0);
        assert_eq!(field.temperature_k(-1.0), 0.0);
        assert_eq!(Field { area_m2: 0.0, ..field }.temperature_k(1.0), f64::INFINITY);
    }

    #[test]
    fn modes_absorb_their_fraction() {
        assert_eq!(Mode::Clear.absorptivity(0.3), 0.3);
        assert_eq!(Mode::Black.absorptivity(0.3), 1.0);
        assert_eq!(Burst::Arriving(10.0).heat_j(0.3), 3.0);
        assert_eq!(Burst::Vent(10.0).heat_j(0.3), 10.0);
    }

    #[test]
    fn the_closed_form_holds_its_ends() {
        let field = test_field();
        let q0 = 3.0e25;
        let p = 5.0e19;
        assert_eq!(field.heat_after_j(q0, p, 0.0), q0);
        assert!(close(field.heat_after_j(q0, p, 1.0), q0 + (p - q0 / field.tau_s), 1e-9));
        assert!(close(field.heat_after_j(q0, p, 1e3 * field.tau_s), p * field.tau_s, 1e-12));
        assert!(close(field.heat_after_j(q0, 0.0, field.tau_s), q0 / std::f64::consts::E, 1e-12));
    }

    #[test]
    fn a_threshold_is_reached_on_time_or_never() {
        let field = test_field();
        let p = 5.0e19;
        let eq = p * field.tau_s;
        let rise = field.time_to_rise_s(0.0, 0.5 * eq, p).unwrap();
        assert!(close(rise, field.tau_s * 2f64.ln(), 1e-12));
        assert!(close(field.heat_after_j(0.0, p, rise), 0.5 * eq, 1e-12));
        let fall = field.time_to_fall_s(eq, 0.5 * eq, 0.0).unwrap();
        assert!(close(fall, field.tau_s * 2f64.ln(), 1e-12));

        assert_eq!(field.time_to_rise_s(eq, 0.5 * eq, p), Some(0.0));
        assert_eq!(field.time_to_fall_s(0.1 * eq, 0.5 * eq, p), Some(0.0));
        assert_eq!(field.time_to_rise_s(0.0, eq, p), None);
        assert_eq!(field.time_to_rise_s(0.0, 2.0 * eq, p), None);
        assert_eq!(field.time_to_fall_s(2.0 * eq, eq, p), None);
        // An equilibrium a hair past the threshold is a long wait, never NaN.
        let hair = field.time_to_rise_s(0.0, eq * (1.0 - 1e-15), p).unwrap();
        assert!(hair.is_finite() && hair > 30.0 * field.tau_s);
        assert_eq!(field.time_to_rise_s(0.0, 1.0, f64::MIN_POSITIVE), None);
        // A threshold a hair ahead is about gap over rate, not a canceled logarithm.
        let near = field.time_to_rise_s(0.0, 1.0, p).unwrap();
        assert!(close(near, 1.0 / p, 1e-9), "{near}");
    }

    #[test]
    fn full_storage_holds_full_and_the_draw_delays_filling() {
        let b = Balance::DEFAULT;
        let me = me(&b);
        let field = test_field();
        let open = Start::new(&b).segment(&b, 0.1, me);
        assert_eq!(open.converted_w(), open.absorbed_w());
        let full = Segment { room_j: 0.0, ..open };
        assert_eq!(full.fill_s(), Some(0.0));
        let settled = field.settle(&full, 0.0, 1.0e5);
        assert_eq!(settled.storage_j, 0.0);
        assert_eq!(field.heat_full_w(&full), full.absorbed_w());
        // A draw that outruns conversion empties even full storage, which converts all segment long.
        let starved = Segment { draw_w: 2.0 * open.stored_w(), ..full };
        assert_eq!(starved.fill_s(), None);
        let settled = field.settle(&starved, 0.0, 1.0e5);
        assert!(close(settled.storage_j, -starved.stored_w() * 1.0e5, 1e-12));
        assert!(close(settled.heat_j, field.heat_after_j(0.0, field.heat_filling_w(&starved), 1.0e5), 1e-12));
        let over = Segment { rating_w: 0.25 * open.absorbed_w(), ..open };
        assert_eq!(over.converted_w(), over.rating_w);
        assert!(close(open.fill_s().unwrap(), me / (open.stored_w() - open.draw_w), 1e-15));
    }

    /// Under the rating with storage empty, sustained power adds only conversion's loss; the same
    /// energy as a burst adds all of it.
    #[test]
    fn a_burst_is_all_heat_and_a_beam_under_the_rating_is_not() {
        let field = test_field();
        let beam = Segment {
            arriving_w: 1.0e19,
            absorptivity: 1.0,
            internal_w: 0.0,
            rating_w: 1.0e20,
            efficiency: 0.7,
            room_j: 1.0e40,
            draw_w: 0.0,
            emitted_w: 0.0,
        };
        let dt_s = 1.0;
        let sustained = field.settle(&beam, 0.0, dt_s).heat_j;
        let burst = Burst::Arriving(beam.arriving_w * dt_s).heat_j(beam.absorptivity);
        assert!(close(sustained, 0.3 * burst, 1e-6), "{sustained} {burst}");
    }

    /// An independent stepper: RK4 on `dQ/dt = P − X − Q/τ`, with storage tracked on its own and
    /// never let past capacity. Storage at capacity takes in only what the draw takes out, and heat
    /// the emission would take below zero is taken from storage instead.
    struct Stepper {
        heat_j: f64,
        stored_j: f64,
        capacity_j: f64,
        t_s: f64,
        filled_at_s: Option<f64>,
    }

    impl Stepper {
        fn run(&mut self, field: &Field, s: &Segment, dt_s: f64, steps: usize, mut stop: impl FnMut(f64) -> bool) -> Option<f64> {
            let h = dt_s / steps as f64;
            let rate = |q: f64, p: f64| p - q / field.tau_s;
            for _ in 0..steps {
                let absorbed = s.arriving_w * s.absorptivity;
                let mut into_storage = s.efficiency * absorbed.min(s.rating_w);
                if self.stored_j >= self.capacity_j {
                    into_storage = into_storage.min(s.draw_w);
                }
                let p = absorbed - into_storage + s.internal_w - s.emitted_w;
                let q = self.heat_j;
                let k1 = rate(q, p);
                let k2 = rate(q + 0.5 * h * k1, p);
                let k3 = rate(q + 0.5 * h * k2, p);
                let k4 = rate(q + h * k3, p);
                self.heat_j = q + h / 6.0 * (k1 + 2.0 * k2 + 2.0 * k3 + k4);
                self.stored_j += (into_storage - s.draw_w) * h + self.heat_j.min(0.0);
                self.heat_j = self.heat_j.max(0.0);
                self.t_s += h;
                if self.stored_j >= self.capacity_j {
                    self.stored_j = self.capacity_j;
                    self.filled_at_s.get_or_insert(self.t_s);
                }
                if stop(self.heat_j) {
                    return Some(self.t_s);
                }
            }
            None
        }
    }

    #[test]
    fn the_closed_form_agrees_with_stepping_through_starlight_bursts_and_a_fill() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let field = start.field;
        let me = me(&b);
        let capacity_j = 30.0 * me;
        let idle_j = start.caps.drain_w * field.tau_s;
        let mut stepper =
            Stepper { heat_j: idle_j, stored_j: 0.0, capacity_j, t_s: 0.0, filled_at_s: None };
        let steps_per_s = 1.0 / 5.0;

        // Starlight at 0.05 AU, not yet full.
        let first = start.segment(&b, 0.05, capacity_j);
        let first_s = 2.0e6;
        let settled = field.settle(&first, idle_j, first_s);
        stepper.run(&field, &first, first_s, (first_s * steps_per_s) as usize, |_| false);
        assert_eq!(settled.filled_s, None);
        assert!(close(settled.heat_j, stepper.heat_j, 1e-6), "{} {}", settled.heat_j, stepper.heat_j);
        let stored_j = settled.storage_j;
        assert!(close(stored_j, stepper.stored_j, 1e-6));

        // A 5 ME vent, and a 3 ME pulse arriving at a Clear field.
        let clear = Mode::Clear.absorptivity(b.clear_absorptivity);
        let heat_j = settled.heat_j + Burst::Vent(5.0 * me).heat_j(clear) + Burst::Arriving(3.0 * me).heat_j(clear);
        stepper.heat_j += 5.0 * me + b.clear_absorptivity * 3.0 * me;

        // Closer in, where storage fills partway and the full field goes past its rated load.
        let second = Segment { arriving_w: start.starlight_w(0.045), room_j: capacity_j - stored_j, ..first };
        let second_s = 8.0e6;
        let settled = field.settle(&second, heat_j, second_s);
        let collapse_s = field.segment_time_to_rise_s(&second, heat_j, field.heat_max_j()).unwrap();
        let heat_max_j = field.heat_max_j();
        let stepped_collapse_s =
            stepper.run(&field, &second, second_s, (second_s * steps_per_s) as usize, |q| q >= heat_max_j);

        let filled_s = settled.filled_s.expect("fills within the segment");
        let stepped_fill_s = stepper.filled_at_s.unwrap() - first_s;
        assert!((filled_s - stepped_fill_s).abs() <= 1.0 / steps_per_s, "{filled_s} {stepped_fill_s}");
        assert!(collapse_s > filled_s && collapse_s < second_s, "{collapse_s}");
        let stepped_collapse_s = stepped_collapse_s.unwrap() - first_s;
        // The stepper spends the step it fills in at the filling power, and near the threshold the
        // heat climbs slowly, so its crossing lags by a little over a step.
        assert!((collapse_s - stepped_collapse_s).abs() <= 2.0 / steps_per_s, "{collapse_s} {stepped_collapse_s}");

        let at_collapse = field.settle(&second, heat_j, collapse_s).heat_j;
        assert!(close(at_collapse, heat_max_j, 1e-9));
        let mut rest = stepper;
        rest.run(&field, &second, second_s - stepped_collapse_s, ((second_s - stepped_collapse_s) * steps_per_s) as usize, |_| false);
        assert!(close(settled.heat_j, rest.heat_j, 1e-6), "{} {}", settled.heat_j, rest.heat_j);
        assert!(close(stored_j + settled.storage_j, rest.stored_j, 1e-9), "{} {}", stored_j + settled.storage_j, rest.stored_j);
    }

    /// Each path through the floor, against the stepper.
    #[test]
    fn the_floor_split_agrees_with_stepping() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let field = start.field;
        let me = me(&b);
        let open = start.segment(&b, 0.1, 100.0 * me);
        let (made_w, filling_w) = (field.heat_filling_w(&open), open.stored_w() - open.draw_w);
        let cases = [
            ("drains", Segment { emitted_w: 3.0 * made_w, ..open }, [false, false]),
            ("fills at the floor", Segment { emitted_w: made_w + 0.5 * filling_w, room_j: 0.2 * me, ..open }, [false, true]),
            ("from full", Segment { emitted_w: 2.0 * field.heat_full_w(&open), room_j: 0.0, ..open }, [true, false]),
        ];
        for (case, segment, [starts_full, ends_full]) in cases {
            let heat_j = 0.05 * me;
            let floor_s = field.segment_time_to_fall_s(&segment, heat_j, 0.0).unwrap();
            let dt_s = 4.0 * floor_s + if ends_full { 2.0 * segment.room_j / (made_w + filling_w - segment.emitted_w) } else { 0.0 };
            let stretches = field.stretches(&segment, heat_j, dt_s);
            assert_eq!(stretches.len(), if ends_full { 3 } else { 2 }, "{case}: premise {stretches:?}");
            assert_eq!((stretches[0].full, stretches[2.min(stretches.len() - 1)].full), (starts_full, ends_full), "{case}: premise");
            assert_eq!(floor_s, stretches[0].dt_s, "{case}");

            let capacity_j = 200.0 * me;
            let stored_j = capacity_j - segment.room_j;
            let mut stepper = Stepper { heat_j, stored_j, capacity_j, t_s: 0.0, filled_at_s: None };
            let steps = 400_000;
            let stepped_floor_s = stepper.run(&field, &segment, dt_s, steps, |q| q <= 0.0).unwrap();
            let h = dt_s / steps as f64;
            assert!((stepped_floor_s - floor_s).abs() <= 2.0 * h, "{case}: {stepped_floor_s} {floor_s}");
            stepper.run(&field, &segment, dt_s - stepped_floor_s, (dt_s - stepped_floor_s).div_euclid(h) as usize, |_| false);
            let settled = field.settle(&segment, heat_j, dt_s);
            let tol_j = 1e-5 * (heat_j + segment.emitted_w * dt_s);
            assert!((settled.heat_j - stepper.heat_j).abs() <= tol_j, "{case}: {} {}", settled.heat_j, stepper.heat_j);
            let storage_j = stepper.stored_j - stored_j;
            assert!((settled.storage_j - storage_j).abs() <= tol_j, "{case}: {} {storage_j}", settled.storage_j);
            let spent_j = segment.emitted_w * dt_s;
            assert!(settled.from_heat_j > heat_j && settled.from_heat_j < spent_j, "{case}: {}", settled.from_heat_j);
        }
    }

    /// Settling `2T` in one leap and as two `T`s agree, filling or full, with the draw under
    /// conversion and over it, and emitting enough to reach the floor in the first `T`: the answer
    /// may not depend on where the caller cuts.
    #[test]
    fn where_segments_are_cut_does_not_matter() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let field = start.field;
        let me = me(&b);
        let t_s = 3.0e6;
        let heat_j = 2.0 * me;
        for room_j in [0.0, 0.5 * me, 100.0 * me] {
            for (draw_w, emitted_w) in [
                (start.caps.drain_w, 0.0),
                (3.0 * start.starlight_w(0.1), 0.0),
                (start.caps.drain_w, 2.0 * heat_j / t_s + start.starlight_w(0.1)),
                (start.caps.drain_w, 0.5 * heat_j / t_s + 3.0 * start.starlight_w(0.1)),
            ] {
                let segment = Segment { room_j, draw_w, emitted_w, ..start.segment(&b, 0.1, 0.0) };
                let leap = field.settle(&segment, heat_j, 2.0 * t_s);
                let half = field.settle(&segment, heat_j, t_s);
                assert!(emitted_w == 0.0 || half.heat_j == 0.0, "premise: at the floor by the cut");
                let rest = Segment { room_j: room_j - half.storage_j, ..segment };
                let halves = field.settle(&rest, half.heat_j, t_s);
                let case = format!("room {room_j:e}, draw {draw_w:e}, emitted {emitted_w:e}");
                assert!(close(leap.heat_j, halves.heat_j, 1e-12), "{case}: {} {}", leap.heat_j, halves.heat_j);
                let storage_j = half.storage_j + halves.storage_j;
                assert!((leap.storage_j - storage_j).abs() <= 1e-12 * me, "{case}: {} {storage_j}", leap.storage_j);
            }
        }
    }

    #[test]
    fn a_fall_across_the_fill_is_found_after_it() {
        let field = test_field();
        // Hot, cooling while it fills, then heating once full.
        let segment = Segment {
            arriving_w: 1.0e19,
            absorptivity: 1.0,
            internal_w: 0.0,
            rating_w: 1.0e20,
            efficiency: 0.9,
            room_j: 1.0e25,
            draw_w: 0.0,
            emitted_w: 0.0,
        };
        let heat_j = 2.0e25;
        let fill_s = segment.fill_s().unwrap();
        let low_j = field.settle(&segment, heat_j, fill_s).heat_j;
        let fall = field.segment_time_to_fall_s(&segment, heat_j, 0.5 * (heat_j + low_j)).unwrap();
        assert!(fall < fill_s);
        assert_eq!(field.segment_time_to_fall_s(&segment, heat_j, 0.9 * low_j), None);
        let full_j = field.equilibrium_j(field.heat_full_w(&segment));
        let rise = field.segment_time_to_rise_s(&segment, low_j, 0.5 * (low_j + full_j)).unwrap();
        assert!(rise > fill_s);
    }

    fn still() -> crate::motion::ShipState {
        crate::motion::ShipState::at(glam::DVec3::ZERO)
    }

    fn starting(b: Balance, stored_j: f64) -> crate::fitting::Fitting {
        use crate::fitting::{Account, Fitting};
        let full = Fitting::full(Form::starting(), b, 0.0);
        Fitting::from_account(&Account { stored_j, ..full.account() }, b)
    }

    /// The account's own settlement, sampled finely, first reaches `Q_max` where the walk says:
    /// storage fills on the way, so the fill split is crossed first.
    #[test]
    fn a_ship_collapses_where_its_settled_heat_first_reaches_q_max() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let mut fitting = starting(b, start.caps.storage_j - 2.0 * me(&b));
        fitting.set_starlight_w(start.starlight_w(0.03));
        let heat_max_j = fitting.field().heat_max_j();
        let collapse_s = fitting.collapse_s(&still(), f64::INFINITY).expect("inside the rated load, it collapses");
        let rest = crate::motion::ShipState::at(glam::DVec3::ZERO);
        assert_eq!(fitting.stored_j_at(&rest, collapse_s), start.caps.storage_j, "premise: full first");
        assert!(close(fitting.heat_j_at(&still(), collapse_s), heat_max_j, 1e-9));

        let n = 100_000;
        let dt_s = 1.5 * collapse_s / n as f64;
        let first = (1..=n).find(|&k| fitting.heat_j_at(&still(), k as f64 * dt_s) >= heat_max_j).unwrap();
        assert!((first - 1) as f64 * dt_s < collapse_s && collapse_s <= first as f64 * dt_s, "{collapse_s} at step {first}");
    }

    #[test]
    fn a_ship_whose_equilibrium_is_short_of_q_max_never_collapses() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let mut fitting = starting(b, start.caps.storage_j);
        fitting.set_starlight_w(start.starlight_w(0.1));
        assert_eq!(fitting.collapse_s(&still(), f64::INFINITY), None);
        fitting.set_starlight_w(start.starlight_w(RATED_LOAD_AU * (1.0 + 1e-6)));
        assert_eq!(fitting.collapse_s(&still(), f64::INFINITY), None);
    }

    /// A vent jumps `Q`, so the crossing is the end of the step that frees it, to the second.
    #[test]
    fn a_vent_that_crosses_q_max_collapses_at_the_end_of_its_step() {
        use crate::form::PartId;
        use crate::refit::rounds::Round;
        let begun = |b: Balance| {
            let mut fitting = crate::fitting::Fitting::full(Form::starting(), b, 0.0);
            fitting.drain(me(&b));
            let mut shrunk = Form::starting();
            shrunk.parts.iter_mut().find(|p| p.id == PartId(2)).unwrap().volume_m3 *= 3.0 / 5.0;
            let round = Round { from: Form::starting(), target: shrunk, stored_j: fitting.stored_j_at(&crate::motion::ShipState::at(glam::DVec3::ZERO), 0.0), start_s: 0.0 };
            let plan = round.solve(&b).unwrap();
            fitting.begin_refit(plan.clone());
            (fitting, plan)
        };
        let (probe, plan) = begun(Balance::DEFAULT);
        let [step] = plan.steps() else { panic!("{:?}", plan.steps()) };
        assert!(step.vented_j > 0.0, "premise: it vents");
        let end_s = step.ends_s();
        let heat_max_j = probe.heat_j_at(&still(), end_s) - 0.5 * step.vented_j;
        assert!(probe.heat_j_at(&still(), end_s * (1.0 - 1e-12)) < heat_max_j, "premise: only the vent crosses");

        let b = Balance { field_capacity: heat_max_j / probe.field().area_m2, ..Balance::DEFAULT };
        let (fitting, _) = begun(b);
        assert!(close(fitting.field().heat_max_j(), heat_max_j, 1e-12));
        assert_eq!(fitting.collapse_s(&still(), f64::INFINITY), Some(end_s));
    }

    /// A build held out of storage under starlight too weak for the drain: free storage runs out a
    /// fifth of the way in and the drain starves, and the collapse is found where the settled heat
    /// first reaches `Q_max`, after that.
    #[test]
    fn a_collapse_after_the_drain_starves_is_where_the_settled_heat_reaches_it() {
        use crate::fitting::{Account, Fitting};
        use crate::form::PartId;
        use crate::refit::rounds::Round;
        let mut target = Form::starting();
        target.parts.iter_mut().find(|p| p.id == PartId(2)).unwrap().volume_m3 *= 7.0 / 5.0;
        let begun = |b: Balance, stored_j: f64| {
            let full = Fitting::full(Form::starting(), b, 0.0);
            let mut fitting = Fitting::from_account(&Account { stored_j, heat_j: 0.0, ..full.account() }, b);
            let drain_w = fitting.hull().capacities.drain_w;
            fitting.set_starlight_w(0.5 * drain_w / b.conversion_efficiency);
            let round = Round { from: Form::starting(), target: target.clone(), stored_j, start_s: 0.0 };
            let plan = round.solve(&b).unwrap();
            fitting.begin_refit(plan.clone());
            (fitting, plan)
        };
        let b = Balance::DEFAULT;
        let (trial, plan) = begun(b, 30.0 * me(&b));
        let cost_j = -plan.steps().iter().map(|s| s.stored_j).sum::<f64>();
        let duration_s = plan.duration_s();
        let spare_j = 0.1 * trial.hull().capacities.drain_w * duration_s;
        let (probe, _) = begun(b, cost_j + spare_j);
        let heat_max_j = probe.heat_j_at(&still(), 0.901_7 * duration_s);
        assert!(probe.heat_j_at(&still(), 0.2 * duration_s) < heat_max_j, "premise: still rising once starved");

        let tight = Balance { field_capacity: heat_max_j / probe.field().area_m2, ..b };
        let (fitting, _) = begun(tight, cost_j + spare_j);
        let collapse_s = fitting.collapse_s(&still(), f64::INFINITY).expect("it reaches the limit");
        let n = 20_000;
        let dt_s = duration_s / n as f64;
        let first = (1..=n).find(|&k| fitting.heat_j_at(&still(), k as f64 * dt_s) >= heat_max_j).unwrap();
        // The limit is the heat at a sample instant, so the solve may land a rounding past it.
        let sampled_s = first as f64 * dt_s * (1.0 + 1e-12);
        assert!((first - 1) as f64 * dt_s < collapse_s && collapse_s <= sampled_s, "{collapse_s} at step {first}");
        assert!(collapse_s > 0.5 * duration_s, "premise: in the starved stretch");
    }

    #[test]
    fn a_field_already_past_q_max_collapses_at_once() {
        let b = Balance::DEFAULT;
        let fitting = starting(b, 0.0);
        let hot = crate::fitting::Account { heat_j: 2.0 * fitting.field().heat_max_j(), since_s: 5.0, ..fitting.account() };
        assert_eq!(crate::fitting::Fitting::from_account(&hot, b).collapse_s(&still(), f64::INFINITY), Some(5.0));
    }

    /// 30's lethal radii: a full starting ship, and one ten and a hundred times its size, collapsing
    /// beside an idle Black starting ship broadside to it.
    #[test]
    fn the_lethal_radii_are_thirtys() {
        let b = Balance::DEFAULT;
        let victim = Start::new(&b);
        let headroom_j = victim.field.heat_max_j() - victim.caps.drain_w * b.field_tau_s;
        for (scale, want_m) in [(1.0, 150.0), (10.0, 4_200.0), (100.0, 130_000.0)] {
            let form = crate::form::presets::named("default", scale).unwrap();
            let field = Field::of(FormGrid::new(&form, &b).unwrap().envelope_area_m2(), &b);
            let spike_j = b.collapse_spike_fraction * field.released_j(Capacities::of(&form, &b).storage_j);
            let r_m = lethal_radius_m(1.0, spike_j, victim.broadside_m2, headroom_j);
            assert!(close(r_m, want_m, 0.02), "{scale} times: {r_m} m");
            let clear_m = lethal_radius_m(b.clear_absorptivity, spike_j, victim.broadside_m2, headroom_j);
            assert!(close(clear_m, b.clear_absorptivity.sqrt() * r_m, 1e-12));
        }
    }
}
