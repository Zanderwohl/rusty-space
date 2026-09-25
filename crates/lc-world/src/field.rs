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

    /// Settles `dt_s` of `segment` from `heat_j`, splitting it where storage fills.
    pub fn settle(&self, segment: &Segment, heat_j: f64, dt_s: f64) -> Settled {
        let fill = segment.fill_s().filter(|&t| t < dt_s);
        let before_s = fill.unwrap_or(dt_s);
        let mut heat = self.heat_after_j(heat_j, self.heat_filling_w(segment), before_s);
        if fill.is_some() {
            heat = self.heat_after_j(heat, self.heat_full_w(segment), dt_s - before_s);
        }
        let storage_j = (segment.stored_w() - segment.draw_w) * before_s;
        Settled { heat_j: heat, storage_j, filled_s: fill }
    }

    /// When heat first reaches `threshold_j` from below under `segment` left running, fill split
    /// included. What a collapse is scheduled from.
    pub fn segment_time_to_rise_s(&self, segment: &Segment, heat_j: f64, threshold_j: f64) -> Option<f64> {
        self.segment_time_to(segment, heat_j, |field, heat, power| {
            field.time_to_rise_s(heat, threshold_j, power)
        })
    }

    /// When heat first falls to `threshold_j` under `segment` left running, fill split included.
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
        let filling = self.heat_filling_w(segment);
        let Some(fill_s) = segment.fill_s() else {
            return time_to(self, heat_j, filling);
        };
        if let Some(t) = time_to(self, heat_j, filling).filter(|&t| t <= fill_s) {
            return Some(t);
        }
        let at_fill = self.heat_after_j(heat_j, filling, fill_s);
        time_to(self, at_fill, self.heat_full_w(segment)).map(|t| fill_s + t)
    }
}

/// Constant inputs to the field, as far as the next change of any of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// Arriving at the field from outside, before absorptivity: starlight, beams, a neighbor's glow.
    pub arriving_w: f64,
    /// [`Mode::absorptivity`].
    pub absorptivity: f64,
    /// Made inside the ship, and all of it heat: the living drain, the drive below ε = 1, what
    /// dismantling loses. Not conversion's loss, which the segment works out itself.
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settled {
    pub heat_j: f64,
    /// Net change in storage: conversion less the draw. Negative when the draw outruns conversion,
    /// and emptying storage is the caller's to handle.
    pub storage_j: f64,
    /// When storage filled, if it did within the step.
    pub filled_s: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fitting::{FIELD_ANCHOR_ME, RATED_LOAD_AU, SOLAR_ANCHOR_AU, SOLAR_ANCHOR_S, STARTING_ENVELOPE_M2};
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
        /// 20's anchor solved on the shadow's broadside, as F10 re-anchors `solar_gain`: the gain
        /// is on the star, so this is what the field takes in once H3 lands.
        gain: f64,
    }

    impl Start {
        fn new(b: &Balance) -> Start {
            let form = Form::starting();
            let grid = FormGrid::new(&form, b).unwrap();
            let caps = Capacities::of(&form, b);
            let broadside_m2 = grid.broadside_m2();
            let wanted_w = caps.storage_j / SOLAR_ANCHOR_S + caps.drain_w;
            let collected_w = b.conversion_efficiency * flux_w_m2(SOLAR_ANCHOR_AU) * broadside_m2;
            Start { field: Field::of(grid.envelope_area_m2(), b), caps, broadside_m2, gain: wanted_w / collected_w }
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

    /// The gain cancels the broadside, so the shadow's starlight is what `solar` collects on the old
    /// ovoid today, and re-anchoring the gain in F10 moves no anchor.
    #[test]
    fn the_starlight_anchored_on_is_todays() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let luminosity_w = SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * AU_M * AU_M;
        let today_w = solar::power_w(&b, 500.0, luminosity_w, RATED_LOAD_AU * AU_M) / b.conversion_efficiency;
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
        assert!((field.temperature_k(b.living_drain_w * field.tau_s) - 400.0).abs() < 1e-9);

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

        let idle_j = b.living_drain_w * field.tau_s;
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
            let living_w = 2.0 * scale.powi(3) * b.living_drain_w;
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
        };
        let dt_s = 1.0;
        let sustained = field.settle(&beam, 0.0, dt_s).heat_j;
        let burst = Burst::Arriving(beam.arriving_w * dt_s).heat_j(beam.absorptivity);
        assert!(close(sustained, 0.3 * burst, 1e-6), "{sustained} {burst}");
    }

    /// An independent stepper: RK4 on `dQ/dt = P − Q/τ`, with storage tracked on its own and never
    /// let past capacity. Storage at capacity takes in only what the draw takes out.
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
                let p = absorbed - into_storage + s.internal_w;
                let q = self.heat_j;
                let k1 = rate(q, p);
                let k2 = rate(q + 0.5 * h * k1, p);
                let k3 = rate(q + 0.5 * h * k2, p);
                let k4 = rate(q + h * k3, p);
                self.heat_j = q + h / 6.0 * (k1 + 2.0 * k2 + 2.0 * k3 + k4);
                self.stored_j += (into_storage - s.draw_w) * h;
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
        let idle_j = b.living_drain_w * field.tau_s;
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

    /// Settling `2T` in one leap and as two `T`s agree, filling or full, with the draw under
    /// conversion and over it: the answer may not depend on where the caller cuts.
    #[test]
    fn where_segments_are_cut_does_not_matter() {
        let b = Balance::DEFAULT;
        let start = Start::new(&b);
        let field = start.field;
        let me = me(&b);
        let t_s = 3.0e6;
        let heat_j = 2.0 * me;
        for room_j in [0.0, 0.5 * me, 100.0 * me] {
            for draw_w in [b.living_drain_w, 3.0 * start.starlight_w(0.1)] {
                let segment = Segment { room_j, draw_w, ..start.segment(&b, 0.1, 0.0) };
                let leap = field.settle(&segment, heat_j, 2.0 * t_s);
                let half = field.settle(&segment, heat_j, t_s);
                let rest = Segment { room_j: room_j - half.storage_j, ..segment };
                let halves = field.settle(&rest, half.heat_j, t_s);
                let case = format!("room {room_j:e}, draw {draw_w:e}");
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
}
