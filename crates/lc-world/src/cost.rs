//! What flying costs: the drive as a rocket whose exhaust is stored energy.
//!
//! `m' = m exp(−Δη/ε)`, where `Δη = ∫ α dτ` is rapidity the drive has put in, summed without
//! regard to direction. Additive in `Δη`, so a burn cut into pieces costs what the whole does and
//! a turn costs what a speed-up does. `lightcone/docs/19-ship-fitting.md` says why kinetic energy
//! was the wrong measure.
//!
//! Every plan holds a constant proper acceleration: the engines throttle down as the ship
//! lightens, so none of the trajectory algebra changes.

use glam::DVec3;

use crate::fitting::C2;
use crate::motion::{Motive, ShipState};

/// Energy spent putting `rapidity` into a ship of `mass_kg`, joules.
pub fn energy_j(mass_kg: f64, rapidity: f64, efficiency: f64) -> f64 {
    // `expm1`: an in-system burn is a rapidity of 1e-4 and `1 - exp` would keep four digits.
    -mass_kg * C2 * (-rapidity / efficiency).exp_m1()
}

/// The most rapidity `free_j` buys a ship of `mass_kg`. Infinite when it buys the ship itself.
pub fn affordable_rapidity(mass_kg: f64, free_j: f64, efficiency: f64) -> f64 {
    let fraction = free_j / (mass_kg * C2);
    if fraction >= 1.0 {
        return f64::INFINITY;
    }
    -efficiency * (-fraction.max(0.0)).ln_1p()
}

/// The rapidity of one velocity seen from another: what an instant change between them costs.
pub fn rapidity_between(from: DVec3, to: DVec3) -> f64 {
    let relative = crate::boost::velocity_to_frame(to, from).length();
    relative.min(crate::flight::MAX_BETA).atanh()
}

/// Rapidity the motive has put in by `now_s`, since it began.
///
/// An escort alongside a burning quarry burns for as long as the quarry does, which no plan
/// bounds: its quarry's acceleration is counted as well as the approach, by magnitude, which
/// overstates a push that is partly canceled by the approach and never understates one.
pub fn lit_rapidity(state: &ShipState, now_s: f64) -> f64 {
    match &state.motive {
        Motive::Crossing(cruise) => cruise.lit_rapidity_at(now_s),
        Motive::Transfer(transfer) => transfer.cruise.lit_rapidity_at(now_s),
        Motive::Consort(plan) => plan.cruise.lit_rapidity_at(now_s),
        Motive::Rendezvous(plan) => {
            plan.cruise.lit_rapidity_at(plan.frame_time_at(now_s - plan.since_t))
        }
        Motive::Escort(plan) => {
            let tau = plan.quarry.tau_at(now_s);
            let keeping = plan.quarry.accel.length() * (tau - plan.cruise.start_s).max(0.0);
            plan.cruise.lit_rapidity_at(tau) + keeping
        }
        // A station is held against milligravities; see `motion::thrust_g`.
        Motive::Holding(_) | Motive::Falling(_) | Motive::Drifting { .. } => 0.0,
    }
}

/// [`lit_rapidity`] once the motive's plan is over. An escort's keeping station is not in it.
pub fn planned_rapidity(state: &ShipState) -> f64 {
    match &state.motive {
        Motive::Crossing(cruise) => cruise.planned_rapidity(),
        Motive::Transfer(transfer) => transfer.cruise.planned_rapidity(),
        Motive::Consort(plan) => plan.cruise.planned_rapidity(),
        Motive::Rendezvous(plan) => plan.cruise.planned_rapidity(),
        Motive::Escort(plan) => plan.cruise.planned_rapidity(),
        Motive::Holding(_) | Motive::Falling(_) | Motive::Drifting { .. } => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fitting::{Balance, Fitting, Loadout};
    use crate::flight::{C_M_S, Cruise, Drive, G0, JULIAN_YEAR_S};

    fn mass() -> f64 {
        let b = Balance::DEFAULT;
        b.dry_mass_kg(&Loadout::STARTING) + b.capacity_j(&Loadout::STARTING) / C2
    }

    #[test]
    fn a_burn_cut_into_pieces_costs_what_the_whole_does() {
        let (m, eta) = (mass(), 0.549);
        let whole = energy_j(m, eta, 1.0);
        let mut left = m;
        for _ in 0..100 {
            left -= energy_j(left, eta / 100.0, 1.0) / C2;
        }
        let pieces = (m - left) * C2;
        assert!((pieces / whole - 1.0).abs() < 1.0e-12, "{pieces} vs {whole}");
    }

    #[test]
    fn at_low_speed_the_cost_is_linear_in_the_change_of_velocity() {
        let (m, dv) = (mass(), 30_000.0);
        for efficiency in [0.5, 1.0, 3.0] {
            let cost = energy_j(m, rapidity_between(DVec3::ZERO, DVec3::X * dv / C_M_S), efficiency);
            let linear = m * dv * C_M_S / efficiency;
            // Second order is `Δv / 2cε`, 1e-4 at the harshest of these.
            assert!((cost / linear - 1.0).abs() < 1.1 * dv / C_M_S / efficiency, "{cost} vs {linear}");
        }
    }

    #[test]
    fn an_efficiency_above_one_is_cheaper_and_still_a_number() {
        let m = mass();
        let one = energy_j(m, 1.0, 1.0);
        let two = energy_j(m, 1.0, 2.0);
        assert!(two < one && two > 0.0 && two.is_finite());
        assert!(affordable_rapidity(m, one, 2.0) > 1.0);
    }

    #[test]
    fn the_affordable_rapidity_spends_exactly_what_is_free() {
        let m = mass();
        for free in [1.0e20, 1.0e25, 3.0e26] {
            let eta = affordable_rapidity(m, free, 0.99);
            assert!((energy_j(m, eta, 0.99) / free - 1.0).abs() < 1.0e-9);
        }
        assert_eq!(affordable_rapidity(m, m * C2, 1.0), f64::INFINITY);
    }

    #[test]
    fn a_full_starting_ship_crosses_at_about_half_c() {
        let b = Balance::DEFAULT;
        let fitting = Fitting::full(Loadout::STARTING, b, 0.0);
        let dry = b.dry_mass_kg(&Loadout::STARTING);
        let eta = affordable_rapidity(mass(), b.capacity_j(&Loadout::STARTING), 1.0);
        assert!((eta - (mass() / dry).ln()).abs() < 1.0e-12);
        assert!(((eta / 2.0).tanh() - 0.461).abs() < 1.0e-3);
        let _ = fitting;
    }

    /// Checked against the rest profile's own closed forms rather than against itself.
    #[test]
    fn a_crossing_from_rest_puts_in_its_boost_and_its_brake() {
        let drive = Drive { max_beta: 0.5, ..Drive::DEFAULT };
        let to = DVec3::X * 2.0;
        let cruise = Cruise::plan(DVec3::ZERO, to, 0.0, drive);
        let peak = cruise.peak_beta();
        assert!((peak - 0.5).abs() < 1.0e-9, "it never reached its cap: {peak}");
        let expected = 2.0 * peak.atanh();
        let planned = cruise.planned_rapidity();
        assert!((planned / expected - 1.0).abs() < 1.0e-9, "{planned} vs {expected}");

        // Nothing is added across the coast, and half is spent by its end.
        let end = cruise.duration_s();
        let mut coasting = None;
        for k in 1..1000 {
            let t = end * k as f64 / 1000.0;
            if cruise.at(t).phase == crate::flight::Phase::Coast {
                coasting.get_or_insert(t);
                assert!((cruise.lit_rapidity_at(t) / peak.atanh() - 1.0).abs() < 1.0e-9);
            }
        }
        assert!(coasting.is_some(), "a two light-year crossing at half c coasts");
        assert!(cruise.lit_rapidity_at(-1.0) == 0.0);
        assert!((cruise.lit_rapidity_at(end + JULIAN_YEAR_S) - planned).abs() < 1.0e-12);
        let _ = G0;
    }

    #[test]
    fn rapidity_between_two_velocities_ignores_the_frame_they_are_given_in() {
        let a = DVec3::new(0.3, 0.1, 0.0);
        let b = DVec3::new(-0.2, 0.4, 0.1);
        let shift = DVec3::new(0.0, -0.6, 0.2);
        let seen = |v: DVec3| crate::boost::velocity_to_frame(v, shift);
        let direct = rapidity_between(a, b);
        assert!((rapidity_between(seen(a), seen(b)) - direct).abs() < 1.0e-12);
        assert!((rapidity_between(DVec3::ZERO, DVec3::X * 0.5) - 0.5f64.atanh()).abs() < 1.0e-15);
    }
}
