//! Closing on a craft that is under thrust, and staying with it.
//!
//! [`crate::pursuit`] plans in the frame a quarry was seen at rest in, and arrives at rest
//! there. For a quarry that is coasting that is exactly matching it. For one that is burning it
//! is not: the quarry has left that frame before the next sighting arrives, so every plan is a
//! "close the gap and stop" aimed at somewhere the quarry no longer is. Re-solved every tick,
//! each one was short enough to be flown whole — turn, burn, flip, brake — and a pursuer plainly
//! leaving the system was drawn braking twenty times a second. It never gained, either: every
//! plan spent its second half shedding the speed its first half had built.
//!
//! **So the plan is made in the quarry's accelerating frame.** In that frame the quarry holds
//! still, and a pursuer holding station beside it is one thrusting at exactly the quarry's
//! acceleration. What is left over for manoeuvring is the difference: a ten-g ship escorting a
//! five-g one has five g to close with, and closing is an ordinary burn-flip-burn with that.
//! Out in the world the two add back together, and the picture is the one a pilot would expect:
//! full thrust to catch up, easing as the gap closes, and settling at the quarry's own
//! acceleration once alongside. When the quarry is out-pulling half the pursuer's drive the
//! plume never turns round at all — the relative brake is still a forward burn, just a gentler
//! one.
//!
//! **What is exact and what is not.** The quarry's own worldline is exact: constant proper
//! acceleration along a fixed line is a hyperbola, and it is evaluated in closed form at any
//! speed. The pursuer's *offset* from it is treated as ordinary motion in the quarry's
//! instantaneous rest frame, which ignores corrections of order `aL/c²` — about one part in a
//! million for five gravities across two million kilometres. Two further simplifications, both
//! named where they apply: the acceleration's direction is used unrotated in every rest frame,
//! which is exact when a quarry burns along its own velocity (a quarry running for somewhere
//! does); and the drive is held at the quarry's acceleration while the pursuer's nose comes
//! about for a relative flip, rather than going dark, so a pursuer escorting a *gently* burning
//! quarry lights before its nose is round by at most that acceleration.
//!
//! Positions in light-years and velocities as fractions of `c`, like the rest of the world;
//! accelerations in light-seconds per second squared, so the boost algebra keeps `c` at one.

use glam::DVec3;

use crate::boost::{self, Event};
use crate::flight::{self, C_M_S, Cruise, Drive, G0, JULIAN_YEAR_S, MAX_BETA};
use crate::motion::{ShipId, ShipState};
use crate::pursuit::{self, Refused, Sighting};

/// A pursuer taking up station on a quarry under thrust.
///
/// Like [`pursuit::Rendezvous`] it is a *frozen* plan: a sighting, and an acceleration measured
/// from two of them. A client evaluating it learns where the pursuer goes and what the pursuer
/// assumed about the quarry — which is what the pursuer's own eyes told it — and nothing about
/// where the quarry actually went.
#[derive(Clone, Debug, PartialEq)]
pub struct Escort {
    /// The approach in the quarry's frame: offsets from it, with the quarry's own proper time
    /// as the clock and the pursuer's drive *less the quarry's acceleration* as the thrust.
    pub cruise: Cruise,
    /// What the quarry is assumed to be doing.
    pub quarry: Burning,
    pub target: ShipId,
}

/// An escort as the arguments it was solved from: what crosses the wire and goes on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    pub from_ly: DVec3,
    pub beta0: DVec3,
    pub to_ly: DVec3,
    /// When the approach begins, in the quarry's proper seconds since the sighting.
    pub start_s: f64,
    /// The drive the *approach* has: the pursuer's, less the quarry's acceleration.
    pub drive: Drive,
    pub quarry: Burning,
    pub target: ShipId,
}

impl Station {
    /// Solve it. `attitude` is the pursuer's nose in world axes, as [`pursuit::Approach::solve`]
    /// takes it and for the same reason.
    pub fn solve(&self, attitude: DVec3) -> Escort {
        let attitude0 = boost::velocity_to_frame(attitude, self.quarry.beta).normalize_or_zero();
        Escort {
            cruise: Cruise::plan_from(
                self.from_ly,
                self.beta0,
                self.to_ly,
                attitude0,
                self.start_s,
                self.drive,
            ),
            quarry: self.quarry,
            target: self.target,
        }
    }
}

/// The worldline an escort assumes for its quarry: seen at one event, burning ever since.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Burning {
    /// Where it was seen, light-years.
    pub position_ly: DVec3,
    /// How fast it was seen going.
    pub beta: DVec3,
    /// Its proper acceleration, light-seconds per second squared.
    pub accel: DVec3,
    /// Coordinate seconds the light left.
    pub since_t: f64,
}

impl Burning {
    /// World seconds since the sighting at which the quarry has aged `tau`.
    pub fn world_elapsed(&self, tau: f64) -> f64 {
        let (t, x, _) = hyperbola(self.accel, tau);
        boost::from_frame(Event { t, x }, self.beta).t
    }

    /// The quarry's proper seconds since the sighting, at a world time.
    ///
    /// Monotonic — `dt/dτ = γ cosh(ατ)(1 + β·β')` and neither factor can reach zero — so a
    /// bracket doubled out from the coasting answer and halved a fixed number of times.
    pub fn tau_at(&self, now_s: f64) -> f64 {
        let elapsed = now_s - self.since_t;
        let guess = elapsed / boost::gamma_of(self.beta);
        let mut span = guess.abs().max(1.0);
        let (mut lo, mut hi) = (guess - span, guess + span);
        for _ in 0..64 {
            if self.world_elapsed(lo) <= elapsed && self.world_elapsed(hi) >= elapsed {
                break;
            }
            span *= 2.0;
            lo = guess - span;
            hi = guess + span;
        }
        for _ in 0..INVERT_STEPS {
            let mid = 0.5 * (lo + hi);
            if self.world_elapsed(mid) < elapsed {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }

    /// Where it is and how fast, `tau` of its own seconds after the sighting.
    pub fn at_tau(&self, tau: f64) -> (DVec3, DVec3) {
        let (t, x, beta) = hyperbola(self.accel, tau);
        let world = boost::from_frame(Event { t, x }, self.beta);
        (
            self.position_ly + world.x / JULIAN_YEAR_S,
            boost::velocity_from_frame(beta, self.beta),
        )
    }

    /// Where it is and how fast at a world time.
    pub fn at(&self, now_s: f64) -> (DVec3, DVec3) {
        self.at_tau(self.tau_at(now_s))
    }
}

/// How many halvings pin a proper time from a world time. Fixed rather than to a tolerance, so
/// every machine takes the same steps to the same bits.
const INVERT_STEPS: usize = 96;

/// The quarry at `tau` proper seconds after the sighting, in the frame it was sighted at rest
/// in: time, displacement in light-seconds, and velocity.
///
/// `t' = sinh(ατ)/α` and `x' = (cosh(ατ) − 1)/α`, with the second written as `2 sinh²(ατ/2)/α`
/// because that form has no cancellation however small `ατ` is.
fn hyperbola(accel: DVec3, tau: f64) -> (f64, DVec3, DVec3) {
    let alpha = accel.length();
    if alpha <= 0.0 {
        return (tau, DVec3::ZERO, DVec3::ZERO);
    }
    let along = accel / alpha;
    let rapidity = alpha * tau;
    let half = (0.5 * rapidity).sinh();
    (rapidity.sinh() / alpha, along * (2.0 * half * half / alpha), along * rapidity.tanh())
}

impl Escort {
    /// The arguments this was solved from.
    pub fn recipe(&self) -> Station {
        Station {
            from_ly: self.cruise.from_ly,
            beta0: self.cruise.initial_beta(),
            to_ly: self.cruise.to_ly,
            start_s: self.cruise.start_s,
            drive: self.cruise.drive,
            quarry: self.quarry,
            target: self.target,
        }
    }

    /// Where the pursuer is, and how fast, at a world time.
    ///
    /// The quarry's position plus an offset rather than a transformation of the pursuer
    /// directly, for the numerical reason [`pursuit::Rendezvous::state_at`] gives.
    pub fn state_at(&self, now_s: f64) -> (DVec3, DVec3) {
        let tau = self.quarry.tau_at(now_s);
        let (quarry, quarry_beta) = self.quarry.at_tau(tau);
        let flight = self.cruise.at(tau);
        let offset = boost::separation_in_world(flight.position_ly * JULIAN_YEAR_S, quarry_beta);
        (
            quarry + offset / JULIAN_YEAR_S,
            boost::velocity_from_frame(flight.beta, quarry_beta),
        )
    }

    /// The pursuer's proper acceleration in the quarry's rest frame, light-seconds per second
    /// squared: the quarry's, plus whatever the approach is doing about the gap.
    fn push_at_tau(&self, tau: f64) -> DVec3 {
        let closing = self.cruise.thrust_at(tau) * (self.cruise.drive.accel_g * G0 / C_M_S);
        self.quarry.accel + closing
    }

    /// Which way the drive points at a world time, in world axes. Zero only where nothing is
    /// lit, which for an escort is a quarry that is not accelerating and a gap already closed.
    pub fn thrust_at(&self, now_s: f64) -> DVec3 {
        let tau = self.quarry.tau_at(now_s);
        let push = self.push_at_tau(tau);
        if push.length_squared() <= 0.0 {
            return DVec3::ZERO;
        }
        let (_, quarry_beta) = self.quarry.at_tau(tau);
        boost::velocity_from_frame(push.normalize(), quarry_beta).normalize_or_zero()
    }

    /// How hard the drive is burning, in g. Not the drive's rating: an escort alongside a
    /// quarry burns at the quarry's acceleration, and the plume says so.
    pub fn thrust_g(&self, now_s: f64) -> f64 {
        self.push_at_tau(self.quarry.tau_at(now_s)).length() * C_M_S / G0
    }

    /// What the nose is being asked to do. Along the thrust when anything is lit, and along
    /// the approach otherwise; the turn toward it is [`crate::motion::facing_at`]'s, from the
    /// attitude the ship had when the escort was taken up.
    pub fn aim_at(&self, now_s: f64) -> flight::Aim {
        let thrust = self.thrust_at(now_s);
        let to = if thrust != DVec3::ZERO {
            thrust
        } else {
            let (_, quarry_beta) = self.quarry.at(now_s);
            let aim = self.cruise.aim_at(self.quarry.tau_at(now_s)).to;
            boost::velocity_from_frame(aim, quarry_beta).normalize_or_zero()
        };
        let since_s = self.quarry.since_t + self.quarry.world_elapsed(self.cruise.start_s);
        flight::Aim { to, from: None, since_s }
    }

    /// The crew's seconds since the escort was taken up. Past the approach they age at the
    /// quarry's rate, being at rest beside it.
    pub fn proper_s_at(&self, now_s: f64) -> f64 {
        let tau = self.quarry.tau_at(now_s);
        let end = self.cruise.start_s + self.cruise.duration_s();
        if tau > end {
            self.cruise.at(end).proper_s + (tau - end)
        } else {
            self.cruise.at(tau).proper_s
        }
    }

    pub fn has_closed(&self, now_s: f64) -> bool {
        self.cruise.has_arrived(self.quarry.tau_at(now_s))
    }

    /// How far off the assumed hyperbola a fresh sighting puts the quarry, light-years. Zero for
    /// a quarry that has held its burn, which is the whole difference from a rendezvous: holding
    /// a burn is no longer a reason to re-plan.
    pub fn divergence(&self, seen: &Sighting) -> f64 {
        self.quarry.at(seen.emitted_s).0.distance(seen.position_ly)
    }
}

/// A quarry's proper acceleration, from two sightings of it, in light-seconds per second
/// squared. `None` when the two cannot say.
///
/// **The change in `γβ`, over the world time between them.** Under constant proper
/// acceleration along a line, `dφ/dτ = α` and `dt/dτ = cosh φ`, so `d(sinh φ)/dt = α`: proper
/// velocity grows *linearly in coordinate time*, and the difference of two of them over the
/// interval is exact whatever the speed and however far apart the sightings are.
///
/// The obvious version is not. Averaging the coordinate acceleration over a tick and scaling
/// it up by `γ³` at the newer sighting overshoots by the speed gained during the tick — a few
/// parts in a hundred thousand at a hundredth of `c`, which sounds like nothing and put an
/// escort's quarry forty kilometres from where it was within ten ticks, and growing.
///
/// Measured over one sighting interval, so a quarry that turned round between two sightings is
/// misread for one of them and put right by the next.
pub fn acceleration_of(previous: &Sighting, latest: &Sighting) -> Option<DVec3> {
    let dt = latest.emitted_s - previous.emitted_s;
    if dt <= 0.0 {
        return None;
    }
    let proper = |beta: DVec3| beta * boost::gamma_of(beta);
    Some((proper(latest.beta) - proper(previous.beta)) / dt)
}

/// Plan taking up station on a quarry believed to be accelerating at `accel`.
///
/// Refused as [`Refused::TooFast`] when there is nothing left to close with: a quarry pulling
/// as hard as the pursuer can is one it can follow but never catch, and one at `c` has no frame.
pub fn escort(
    pursuer: &ShipState,
    pursuer_length_m: f64,
    seen: &Sighting,
    accel: DVec3,
    now_s: f64,
    drive: Drive,
) -> Result<Escort, Refused> {
    if seen.beta.length() >= MAX_BETA {
        return Err(Refused::TooFast);
    }
    let spare_g = drive.accel_g - accel.length() * C_M_S / G0;
    if spare_g <= drive.accel_g * SPARE_FLOOR {
        return Err(Refused::TooFast);
    }
    let burning =
        Burning { position_ly: seen.position_ly, beta: seen.beta, accel, since_t: seen.emitted_s };
    // Everything below is measured beside the quarry as it is *now*, on the hyperbola the
    // sighting and the acceleration put it on.
    let tau = burning.tau_at(now_s);
    let (quarry, quarry_beta) = burning.at_tau(tau);
    let separation = (pursuer.position_ly - quarry) * JULIAN_YEAR_S;
    let offset = boost::to_frame(Event { t: 0.0, x: separation }, quarry_beta).x;
    let standoff_ls = pursuit::standoff_m(pursuer_length_m, seen.length_m) / C_M_S;
    // The pursuer's side, as for a rendezvous. A pursuer somehow exactly on the quarry takes
    // station astern of the burn rather than nowhere.
    let side = offset.try_normalize().unwrap_or(-accel.normalize_or(DVec3::X));
    Ok(Station {
        from_ly: offset / JULIAN_YEAR_S,
        beta0: boost::velocity_to_frame(pursuer.beta, quarry_beta),
        to_ly: side * standoff_ls / JULIAN_YEAR_S,
        start_s: tau,
        drive: Drive { accel_g: spare_g, ..drive },
        quarry: burning,
        target: seen.target,
    }
    .solve(pursuer.attitude))
}

/// The least spare thrust worth planning with, as a fraction of the drive. Below it the
/// approach would take so long that "follow at the same acceleration" is the honest answer, and
/// that is not a plan this module makes.
const SPARE_FLOOR: f64 = 0.02;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::M_PER_LY;

    const KM_LY: f64 = 1.0e3 / M_PER_LY;
    /// Five gravities, in the units this module works in.
    const FIVE_G: f64 = 5.0 * G0 / C_M_S;

    fn quarry(at_km: f64, beta: DVec3) -> Sighting {
        Sighting {
            target: ShipId(2),
            position_ly: DVec3::X * at_km * KM_LY,
            beta,
            length_m: 500.0,
            emitted_s: 0.0,
        }
    }

    fn ten_g() -> Drive {
        Drive { accel_g: 10.0, slew_rate_rad_s: 1.0, ..Drive::DEFAULT }
    }

    /// The quarry's worldline is the textbook hyperbola: from rest, after `τ` of its own time,
    /// it is `(cosh ατ − 1)/α` along and moving at `tanh ατ`.
    #[test]
    fn a_burning_quarry_follows_its_hyperbola() {
        let seen = quarry(0.0, DVec3::ZERO);
        let plan = escort(&ShipState::at(DVec3::X * -1.0e4 * KM_LY), 500.0, &seen, DVec3::X * FIVE_G, 0.0, ten_g())
            .expect("a plan");
        let tau = 3.0e6;
        let world_t = plan.quarry.world_elapsed(tau);
        let (at, beta) = plan.quarry.at(world_t);
        let k = FIVE_G * tau;
        let want_ly = (k.cosh() - 1.0) / FIVE_G / JULIAN_YEAR_S;
        assert!((at.x - want_ly).abs() < 1.0e-12 * want_ly.max(1.0), "{} against {want_ly}", at.x);
        assert!((beta.x - k.tanh()).abs() < 1.0e-12, "{} against {}", beta.x, k.tanh());
        // And the world time really is `sinh(ατ)/α`, which is the inversion checked end to end.
        assert!((plan.quarry.tau_at(world_t) - tau).abs() < 1.0e-6 * tau, "{}", plan.quarry.tau_at(world_t));
    }

    /// **The report this exists for**, in the numbers it was put in: a five-g pursuer behind a
    /// four-g quarry. It never points its drive against the quarry's burn. It catches up at
    /// its full five, eases to three for the relative brake — still forward — and settles at
    /// the quarry's own four. No flip, and no brake plume.
    #[test]
    fn chasing_a_harder_burn_never_brakes() {
        let seen = quarry(0.0, DVec3::X * 0.01);
        let pursuer = {
            let mut s = ShipState::at(DVec3::X * -2.0e6 * KM_LY);
            s.beta = DVec3::X * 0.01;
            s.attitude = DVec3::X;
            s
        };
        let four_g = DVec3::X * 4.0 * G0 / C_M_S;
        let five_g_drive = Drive { accel_g: 5.0, ..ten_g() };
        let plan = escort(&pursuer, 500.0, &seen, four_g, 0.0, five_g_drive).expect("a plan");
        let start = plan.quarry.since_t + plan.quarry.world_elapsed(plan.cruise.start_s);
        let end = plan.quarry.since_t + plan.quarry.world_elapsed(plan.cruise.start_s + plan.cruise.duration_s());
        assert!(end > start, "the approach has no length");
        let (mut peak, mut least) = (0.0f64, f64::INFINITY);
        for k in 0..=400 {
            let t = start + (end - start) * k as f64 / 400.0;
            let thrust = plan.thrust_at(t);
            let (_, beta) = plan.state_at(t);
            assert!(thrust.dot(DVec3::X) > 0.0, "braking at {k}/400: {thrust} while moving {beta}");
            let g = plan.thrust_g(t);
            assert!(g <= 5.0 + 1.0e-6, "{g} g is more drive than the ship has");
            peak = peak.max(g);
            least = least.min(g);
        }
        assert!(peak > 4.99, "it never used its spare thrust: peaked at {peak} g");
        assert!((least - 3.0).abs() < 0.01, "the relative brake should ease to three g: {least}");
        // Alongside, it burns at exactly the quarry's four.
        let later = end + 3.0e5;
        assert!((plan.thrust_g(later) - 4.0).abs() < 1.0e-9, "{} g at parity", plan.thrust_g(later));
        assert!(plan.has_closed(later));
    }

    /// Station-keeping is the quarry's own velocity at a standoff from it, for as long as the
    /// quarry keeps burning — which is what "matched" has to mean for something accelerating.
    #[test]
    fn alongside_it_stays_alongside() {
        let seen = quarry(0.0, DVec3::ZERO);
        let pursuer = ShipState::at(DVec3::X * -5.0e4 * KM_LY);
        let plan = escort(&pursuer, 500.0, &seen, DVec3::X * FIVE_G, 0.0, ten_g()).expect("a plan");
        let end = plan.quarry.since_t + plan.quarry.world_elapsed(plan.cruise.start_s + plan.cruise.duration_s());
        let standoff_ly = pursuit::standoff_m(500.0, 500.0) / M_PER_LY;
        for extra in [0.0, 1.0e5, 1.0e6] {
            let t = end + extra;
            let (at, beta) = plan.state_at(t);
            let (quarry_at, quarry_beta) = plan.quarry.at(t);
            let gap_ly = boost::separation_in_frame((at - quarry_at) * JULIAN_YEAR_S, quarry_beta)
                / JULIAN_YEAR_S;
            assert!((gap_ly - standoff_ly).abs() < 1.0e-3 * standoff_ly, "{gap_ly} ly apart after {extra} s");
            let relative = boost::velocity_to_frame(beta, quarry_beta);
            assert!(relative.length() < 1.0e-9, "drifting at {relative} after {extra} s");
        }
    }

    /// A quarry holding its burn is not a reason to re-plan. That is the other half of the fix:
    /// a rendezvous was thrown away every tick against a burning quarry because its frame was
    /// wrong by the next sighting, and an escort's is not.
    #[test]
    fn a_held_burn_does_not_diverge() {
        let seen = quarry(0.0, DVec3::ZERO);
        let plan = escort(&ShipState::at(DVec3::X * -5.0e4 * KM_LY), 500.0, &seen, DVec3::X * FIVE_G, 0.0, ten_g())
            .expect("a plan");
        let later_s = plan.quarry.world_elapsed(2.0e5);
        let (at, beta) = plan.quarry.at(later_s);
        let fresh = Sighting { position_ly: at, beta, emitted_s: later_s, ..seen };
        assert!(plan.divergence(&fresh) * M_PER_LY < 1.0, "{} m", plan.divergence(&fresh) * M_PER_LY);
    }

    /// What two sightings measure is proper acceleration, not the coordinate rate the velocity
    /// changes at, and it has to be exact over a whole tick rather than over a few seconds: a
    /// shard's sightings of a burning quarry are hours of coordinate time apart.
    #[test]
    fn acceleration_is_exact_across_a_long_interval() {
        let seen = quarry(0.0, DVec3::ZERO);
        let plan = escort(&ShipState::at(DVec3::X * -5.0e4 * KM_LY), 500.0, &seen, DVec3::X * FIVE_G, 0.0, ten_g())
            .expect("a plan");
        let sighting_at = |tau: f64| {
            let t = plan.quarry.world_elapsed(tau);
            let (at, beta) = plan.quarry.at(t);
            Sighting { position_ly: at, beta, emitted_s: t, ..seen }
        };
        // A tick at twenty times the design rate apart, early and late in a burn.
        for tau in [2.0e5, 5.0e6] {
            let (a, b) = (sighting_at(tau), sighting_at(tau + 8_766.0));
            let measured = acceleration_of(&a, &b).expect("two sightings");
            assert!(
                (measured.length() / FIVE_G - 1.0).abs() < 1.0e-9,
                "{} of the true acceleration at τ = {tau}",
                measured.length() / FIVE_G,
            );
        }
    }

    /// Near `c` proper and coordinate acceleration are a factor of `γ³` apart.
    #[test]
    fn acceleration_is_read_as_proper() {
        let seen = quarry(0.0, DVec3::ZERO);
        let plan = escort(&ShipState::at(DVec3::X * -5.0e4 * KM_LY), 500.0, &seen, DVec3::X * FIVE_G, 0.0, ten_g())
            .expect("a plan");
        let sighting_at = |tau: f64| {
            let t = plan.quarry.world_elapsed(tau);
            let (at, beta) = plan.quarry.at(t);
            Sighting { position_ly: at, beta, emitted_s: t, ..seen }
        };
        // Deep into the burn, where the quarry is well past half of `c`.
        let (a, b) = (sighting_at(5.0e6), sighting_at(5.0e6 + 10.0));
        assert!(b.beta.length() > 0.5, "premise: fast enough to tell the two apart");
        let measured = acceleration_of(&a, &b).expect("two sightings");
        assert!((measured.length() / FIVE_G - 1.0).abs() < 1.0e-3, "{} g", measured.length() * C_M_S / G0);
    }

    /// A quarry pulling as hard as the pursuer can is followed, never caught, and is refused
    /// rather than planned into an approach that never ends.
    #[test]
    fn a_quarry_that_pulls_as_hard_is_refused() {
        let seen = quarry(0.0, DVec3::ZERO);
        let ten = DVec3::X * 10.0 * G0 / C_M_S;
        let refused = escort(&ShipState::at(DVec3::X * -5.0e4 * KM_LY), 500.0, &seen, ten, 0.0, ten_g());
        assert_eq!(refused.err(), Some(Refused::TooFast));
    }
}
