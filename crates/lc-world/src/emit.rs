//! What an emission delivers: an aperture's rating, the flux along its cone, the share that lands
//! on a receiver, and how sure the aim behind it can be. See
//! `lightcone/docs/31-directed-energy.md`.
//!
//! An emission is a top-hat cone from a point: uniform intensity inside the half-angle and none
//! outside, the same cone [`Beam::covers`](crate::signal::Beam::covers) tests. So flux is inverse
//! square and **infinite at zero distance**, and [`flux_w_m2`] says so rather than inventing a
//! finite number. What lands on a receiver is bounded all the same: [`received_fraction`] is one
//! wherever the spot is no larger than the receiver's shadow, so no receiver ever takes more than
//! was sent. Distances are meters and powers watts; the caller supplies both.

use glam::DVec3;

use serde::{Deserialize, Serialize};

use crate::boost::gamma_of;
use crate::craft::{BEAM_PER_LENGTH, Craft};
use crate::escort::Burning;
use crate::fitting::Balance;
use crate::flight::{Aim, C_M_S, Drive, G0, JULIAN_YEAR_S};
use crate::form::capacity::Aperture;
use crate::motion::{self, Motive};
use crate::signal::cone_solid_angle_sr;

/// What `engine_m3` of engine can send, watts: its exhaust, its deliberate emission and its
/// conversion into storage are all bounded by this.
pub fn rating_w(balance: &Balance, engine_m3: f64) -> f64 {
    balance.engine_density_w * engine_m3.max(0.0)
}

/// The temperature of an aperture's open face, kelvin: a blackbody radiating all of `power_w`
/// through `face_m2`. For a photon drive `power_w` is `F c`.
pub fn aperture_temperature_k(power_w: f64, face_m2: f64) -> f64 {
    if power_w <= 0.0 || face_m2 <= 0.0 {
        return 0.0;
    }
    (power_w / (em_spectra::blackbody::SIGMA * face_m2)).powf(0.25)
}

/// W/m² anywhere inside a cone of `half_angle_rad` carrying `power_w`, at `distance_m`.
pub fn flux_w_m2(power_w: f64, half_angle_rad: f64, distance_m: f64) -> f64 {
    if power_w <= 0.0 {
        return 0.0;
    }
    power_w / (cone_solid_angle_sr(half_angle_rad) * distance_m * distance_m)
}

/// Where [`flux_w_m2`] falls to `flux_w_m2`, meters. Zero for no power, infinite for no flux.
pub fn distance_at_flux_m(power_w: f64, half_angle_rad: f64, flux_w_m2: f64) -> f64 {
    if power_w <= 0.0 {
        return 0.0;
    }
    (power_w / (cone_solid_angle_sr(half_angle_rad) * flux_w_m2)).sqrt()
}

/// The share of an emission a receiver in its cone intercepts: its shadow toward the emitter over
/// the spot, at most one.
///
/// The spot is the cap the cone cuts from a sphere of `distance_m`, not the disk `pi (theta d)^2`
/// that 31's formula writes, which is the same thing only for a narrow cone.
pub fn received_fraction(half_angle_rad: f64, shadow_m2: f64, distance_m: f64) -> f64 {
    if shadow_m2 <= 0.0 {
        return 0.0;
    }
    let spot_m2 = cone_solid_angle_sr(half_angle_rad) * distance_m * distance_m;
    if spot_m2 <= shadow_m2 { 1.0 } else { shadow_m2 / spot_m2 }
}

/// How far from its predicted place a target free to thrust at `accel_m_s2` can be, meters, when
/// the prediction is `blind_s` old at the beam's arrival.
///
/// Hyperbolic motion from rest, `(c²/a)(√(1 + x²) − 1)` with `x = a t / c`, so it never passes
/// `c t`: `½ a t²` does after 71 days at 5 g, and at four light-years says twenty times what light
/// could cover. Written as `a t² / (√(1 + x²) + 1)` because the first form cancels to nothing at
/// the light-seconds 31 tabulates, where `x²` is 10⁻¹³.
pub fn lead_uncertainty_m(accel_m_s2: f64, blind_s: f64) -> f64 {
    let a = accel_m_s2.abs();
    let x = a * blind_s / C_M_S;
    a * blind_s * blind_s / ((1.0 + x * x).sqrt() + 1.0)
}

/// How long a target is unwatched when the freshest sighting of it is aimed at: its light's
/// flight here plus the beam's flight back, seconds.
pub fn blind_s(distance_m: f64) -> f64 {
    2.0 * distance_m / C_M_S
}

/// Which of a ship's two drives a jet leaves: the main drive, or the station-keeping thrusters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Jet {
    Drive,
    Thrusters,
}

/// A lit drive as an emission, at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exhaust {
    pub jet: Jet,
    /// Unit, along the exhaust, world axes.
    pub axis: DVec3,
    /// `F c`, W.
    pub power_w: f64,
    pub half_angle_rad: f64,
}

/// What `craft`'s drives send out at `now_s`, at the mass it has then: the rocket law's throttle.
///
/// Nothing for an emit flown as a burn, which is its own emission. A leg at no more than
/// `rcs_accel_g` is on the thrusters, and an escort's thruster leg is two jets: the main drive
/// carrying the quarry's acceleration, and the thrusters the closing.
pub fn exhaust(craft: &Craft, balance: &Balance, now_s: f64) -> Vec<Exhaust> {
    let state = craft.motion_at(now_s);
    let pushes_g = match &state.motive {
        Motive::Boosting(_) => Vec::new(),
        Motive::Escort(plan) => {
            let (keeping, closing) = plan.pushes_g(now_s);
            if crate::courtesy::on_thrusters(balance, &plan.cruise.drive) {
                vec![(Jet::Drive, keeping), (Jet::Thrusters, closing)]
            } else {
                vec![(Jet::Drive, keeping + closing)]
            }
        }
        _ => {
            let g = motion::thrust_g(state, now_s);
            let jet = if g <= balance.rcs_accel_g { Jet::Thrusters } else { Jet::Drive };
            vec![(jet, motion::thrust_at(state, now_s) * g)]
        }
    };
    let pushes_g: Vec<(Jet, DVec3)> = pushes_g.into_iter().filter(|(_, g)| g.length() > 0.0).collect();
    if pushes_g.is_empty() {
        return Vec::new();
    }
    let mass_kg = craft.mass_kg_at(now_s);
    pushes_g
        .into_iter()
        .map(|(jet, push_g)| Exhaust {
            jet,
            axis: -push_g.normalize(),
            power_w: Drive::exhaust_w(mass_kg, push_g.length()),
            half_angle_rad: if jet == Jet::Drive { balance.drive_spread_rad } else { balance.rcs_spread_rad },
        })
        .collect()
}

/// What `craft`'s main drive sends aft at `now_s`, watts: its [`Jet::Drive`] from [`exhaust`]. What
/// `Presence` states, and what a face and a cone are drawn from. Zero on the thrusters alone.
pub fn drive_w(craft: &Craft, balance: &Balance, now_s: f64) -> f64 {
    exhaust(craft, balance, now_s).iter().filter(|j| j.jet == Jet::Drive).map(|j| j.power_w).sum()
}

/// What an emit flown as a burn sends at `now_s`, W, at the rocket law's throttle, and whether it
/// leaves the fore faces. `None` while it is not lit.
pub fn boost_w(craft: &Craft, now_s: f64) -> Option<(f64, bool)> {
    match &craft.motion_at(now_s).motive {
        Motive::Boosting(boost) if boost.thrust_at(now_s) != DVec3::ZERO => {
            // Recoil is against the beam, so a beam out of the bow pushes the ship backward.
            Some((Drive::exhaust_w(craft.mass_kg_at(now_s), boost.accel_g), boost.thrust.dot(boost.nose) < 0.0))
        }
        _ => None,
    }
}

/// What leaves each end's open faces, W.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ends {
    pub fore_w: f64,
    pub aft_w: f64,
}

impl Ends {
    /// With the main drive's `F c` leaving aft beside it.
    pub fn with_drive(self, drive_w: f64) -> Self {
        Ends { aft_w: self.aft_w + drive_w, ..self }
    }

    /// What leaves through one face: its share of its end's.
    pub fn through(&self, aperture: &Aperture) -> f64 {
        let end_w = if aperture.fore() { self.fore_w } else if aperture.aft() { self.aft_w } else { 0.0 };
        end_w * aperture.share
    }

    /// That face's temperature, K.
    pub fn face_k(&self, aperture: &Aperture) -> f64 {
        aperture_temperature_k(self.through(aperture), aperture.area_m2())
    }

    pub fn is_dark(&self) -> bool {
        self.fore_w <= 0.0 && self.aft_w <= 0.0
    }
}

/// What `craft`'s emits send out of each end at `now_s`: an emit flown as a burn from the end it
/// was ordered from, and a balanced one its `power_w` from each, half what it draws. The drive is not in it. What `Presence`
/// states beside `drive_w`.
pub fn emit_w(craft: &Craft, now_s: f64) -> Ends {
    let balanced = 0.5 * craft.balanced_w_at(now_s);
    let mut ends = Ends { fore_w: balanced, aft_w: balanced };
    match boost_w(craft, now_s) {
        Some((w, true)) => ends.fore_w += w,
        Some((w, false)) => ends.aft_w += w,
        None => {}
    }
    ends
}

/// Everything leaving `craft`'s open faces at `now_s`: its emits, and its main drive aft.
pub fn faces_w(craft: &Craft, balance: &Balance, now_s: f64) -> Ends {
    emit_w(craft, now_s).with_drive(drive_w(craft, balance, now_s))
}

/// The open faces a craft's exhaust leaves through, m²: its aft engines', or an unfitted hull's
/// cross-section.
pub fn exhaust_face_m2(craft: &Craft) -> f64 {
    let faces = craft.fitting().and_then(|f| crate::form::capacity::aft_apertures(f.form(), f.balance()));
    match faces {
        Some(faces) if !faces.is_empty() => faces.iter().map(|a| std::f64::consts::PI * a.radius_m * a.radius_m).sum(),
        _ => std::f64::consts::PI * (0.5 * BEAM_PER_LENGTH * craft.length_m).powi(2),
    }
}

/// An emission with net thrust, flown: the nose comes about to `nose` with nothing lit, then the
/// ship is pushed along `thrust` at a constant proper acceleration for `lit_s` coordinate seconds,
/// and drifts after. Recoil is `thrust`, against the beam, whichever end the beam leaves from.
///
/// The power at a constant acceleration falls as the ship lightens, exactly as the drive throttles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Boost {
    pub from_ly: DVec3,
    pub beta0: DVec3,
    /// Coordinate seconds it was ordered, which is when the turn begins.
    pub start_s: f64,
    /// Unit, world axes.
    pub thrust: DVec3,
    /// Unit: where the nose points while lit.
    pub nose: DVec3,
    pub accel_g: f64,
    pub lit_s: f64,
    turn_s: f64,
}

impl Boost {
    /// `attitude0` is where the nose was when ordered, and `slew_rate_rad_s` how fast it turns.
    #[allow(clippy::too_many_arguments)]
    pub fn plan(
        from_ly: DVec3,
        beta0: DVec3,
        start_s: f64,
        thrust: DVec3,
        nose: DVec3,
        accel_g: f64,
        lit_s: f64,
        attitude0: DVec3,
        slew_rate_rad_s: f64,
    ) -> Self {
        let nose = nose.normalize_or(DVec3::X);
        let turn_s = crate::attitude::turn_time_s(attitude0, nose, slew_rate_rad_s);
        Self { from_ly, beta0, start_s, thrust: thrust.normalize_or(-nose), nose, accel_g, lit_s: lit_s.max(0.0), turn_s }
    }

    /// Exactly as planned, the turn included: what a recipe carries.
    #[allow(clippy::too_many_arguments)]
    pub fn resume(
        from_ly: DVec3,
        beta0: DVec3,
        start_s: f64,
        thrust: DVec3,
        nose: DVec3,
        accel_g: f64,
        lit_s: f64,
        turn_s: f64,
    ) -> Self {
        Self { from_ly, beta0, start_s, thrust, nose, accel_g, lit_s, turn_s }
    }

    pub fn turn_s(&self) -> f64 {
        self.turn_s
    }

    pub fn lights_s(&self) -> f64 {
        self.start_s + self.turn_s
    }

    pub fn out_s(&self) -> f64 {
        self.lights_s() + self.lit_s
    }

    /// Proper acceleration, light-seconds per second squared.
    fn alpha(&self) -> f64 {
        self.accel_g * G0 / C_M_S
    }

    fn burning(&self) -> Burning {
        Burning {
            position_ly: self.from_ly + self.beta0 * (self.turn_s / JULIAN_YEAR_S),
            beta: self.beta0,
            accel: self.thrust * self.alpha(),
            since_t: self.lights_s(),
        }
    }

    /// Where the ship is and how fast, at a coordinate time.
    pub fn state_at(&self, now_s: f64) -> (DVec3, DVec3) {
        if now_s < self.lights_s() {
            let t = (now_s - self.start_s).max(0.0);
            return (self.from_ly + self.beta0 * (t / JULIAN_YEAR_S), self.beta0);
        }
        let burning = self.burning();
        let (at, beta) = burning.at(now_s.min(self.out_s()));
        (at + beta * ((now_s - self.out_s()).max(0.0) / JULIAN_YEAR_S), beta)
    }

    /// Proper seconds since it was ordered.
    pub fn proper_s(&self, now_s: f64) -> f64 {
        let turned = (now_s - self.start_s).clamp(0.0, self.turn_s) / gamma_of(self.beta0);
        if now_s <= self.lights_s() {
            return turned;
        }
        let tau = self.lit_tau(now_s);
        let after = (now_s - self.out_s()).max(0.0) / gamma_of(self.state_at(self.out_s()).1);
        turned + tau + after
    }

    /// The ship's proper seconds lit by `now_s`.
    fn lit_tau(&self, now_s: f64) -> f64 {
        if now_s <= self.lights_s() {
            return 0.0;
        }
        self.burning().tau_at(now_s.min(self.out_s()))
    }

    /// One order, given when it was: turn to `nose` and light.
    pub fn aim_at(&self) -> Aim {
        Aim { to: self.nose, from: None, since_s: self.start_s }
    }

    /// Zero where nothing is lit.
    pub fn thrust_at(&self, now_s: f64) -> DVec3 {
        if now_s >= self.lights_s() && now_s < self.out_s() { self.thrust } else { DVec3::ZERO }
    }

    pub fn lit_rapidity_at(&self, now_s: f64) -> f64 {
        self.alpha() * self.lit_tau(now_s)
    }

    pub fn planned_rapidity(&self) -> f64 {
        self.lit_rapidity_at(self.out_s())
    }

    pub fn has_ended(&self, now_s: f64) -> bool {
        now_s >= self.out_s()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fitting::Fitting;
    use crate::form::presets::SLOT_M3;
    use crate::form::Form;
    use crate::flight::{G0, JULIAN_YEAR_S};
    use crate::signal::Transmitter;
    use crate::solar::broadside_m2;

    const AU_M: f64 = 1.495_978_707e11;
    const LY_M: f64 = C_M_S * JULIAN_YEAR_S;

    #[test]
    fn the_starting_drive_section_is_rated_at_its_five_g() {
        let b = Balance::DEFAULT;
        let drive_m3 = 5.0 * SLOT_M3;
        assert_eq!(format!("{:.2e}", drive_m3), "1.96e6");
        let rated = rating_w(&b, drive_m3);
        assert_eq!(format!("{rated:.1e}"), "1.1e20");
        // The anchor is dry mass and 30 ME, to 1e-9. The field's heat is not in it.
        let fitting = Fitting::full(Form::starting(), b, 0.0);
        let full_kg = fitting.hull().dry_kg + fitting.hull().capacities.storage_j / crate::fitting::C2;
        let five_g = Drive::exhaust_w(full_kg, 5.0);
        assert!((rated - five_g).abs() / rated < 1.0e-9, "{rated} against {five_g}");
    }

    /// 32 §The exhaust cone: the starting drive, all of it through a 100 m face, is white-hot.
    #[test]
    fn the_starting_drive_s_face_is_white_hot() {
        let b = Balance::DEFAULT;
        let rated = rating_w(&b, 5.0 * SLOT_M3);
        let face_m2 = std::f64::consts::PI * 50.0 * 50.0;
        assert_eq!(format!("{:.1e}", aperture_temperature_k(rated, face_m2)), "7.0e5");
        assert_eq!(aperture_temperature_k(0.0, face_m2), 0.0, "an unlit drive has no face");
    }

    /// 31 §What arrives, every cell: a 100 m aperture at its diffraction floor, the spot across,
    /// and the fraction a 500 m, 5 km and 50 km hull collects broadside.
    #[test]
    fn what_arrives_table() {
        let rows: [(f64, f64, &str, [&str; 3]); 5] = [
            (AU_M, 1.0e-6, "1.5e3", ["7e-2", "1e0", "1e0"]),
            (AU_M, 1.0e-9, "1.5e0", ["1e0", "1e0", "1e0"]),
            (100.0 * AU_M, 1.0e-6, "1.5e5", ["7e-6", "7e-4", "7e-2"]),
            (100.0 * AU_M, 1.0e-9, "1.5e2", ["1e0", "1e0", "1e0"]),
            (4.0 * LY_M, 1.0e-9, "3.8e5", ["1e-6", "1e-4", "1e-2"]),
        ];
        for (d, lambda, spot, fractions) in rows {
            let t = Transmitter::new(lambda, 100.0);
            assert_eq!(format!("{:.1e}", t.spot_at(d)), spot, "spot at {d:e} m, {lambda:e} m");
            for (length, want) in [500.0, 5_000.0, 50_000.0].into_iter().zip(fractions) {
                let got = received_fraction(t.half_angle_rad(), broadside_m2(length), d);
                assert_eq!(format!("{got:.0e}"), want, "{length} m hull at {d:e} m, {lambda:e} m");
            }
        }
        // 31 §Dumping heat: 10⁻¹¹ radians wide.
        assert_eq!(format!("{:.0e}", 2.0 * Transmitter::new(1.0e-9, 100.0).half_angle_rad()), "1e-11");
    }

    /// 31 §Spread: a 5 g target, aimed at from its freshest sighting.
    #[test]
    fn lead_uncertainty_table() {
        let a = 5.0 * G0;
        let rows = [(1.0, "1e2"), (10.0, "1e4"), (60.0, "3.5e5")];
        for (light_s, want) in rows {
            let got = lead_uncertainty_m(a, blind_s(light_s * C_M_S));
            let digits = if want.contains('.') { 1 } else { 0 };
            assert_eq!(format!("{got:.digits$e}"), want, "{light_s} light-seconds");
        }
    }

    /// Past a few light-days, a target can be anywhere light could have reached, less the time it
    /// spends getting up to speed: `c t − c²/a` in the limit.
    #[test]
    fn lead_uncertainty_is_bounded_by_light() {
        let a = 5.0 * G0;
        let t = blind_s(4.0 * LY_M);
        let got = lead_uncertainty_m(a, t);
        assert!(got < C_M_S * t, "{got:e} past light's {:e}", C_M_S * t);
        let limit = C_M_S * t - C_M_S * C_M_S / a;
        assert!((got - limit).abs() / limit < 1.0e-3, "{got:e} against {limit:e}");
        // And still ½ a t² where the table lives, to a part in 10⁶.
        let t = blind_s(C_M_S);
        assert!((lead_uncertainty_m(a, t) / (0.5 * a * t * t) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn flux_is_inverse_square_and_power_over_the_spot() {
        let (p, half) = (1.0e20, 5.0_f64.to_radians());
        let near = flux_w_m2(p, half, 1_000.0);
        assert!((near / flux_w_m2(p, half, 2_000.0) - 4.0).abs() < 1.0e-12);
        // Everything sent crosses the spot: flux times the spot's area is the power.
        assert!((near * cone_solid_angle_sr(half) * 1.0e6 - p).abs() / p < 1.0e-12);
        let d = distance_at_flux_m(p, half, near);
        assert!((d - 1_000.0).abs() < 1.0e-9, "{d}");
    }

    /// The decision at zero distance: the flux is honestly infinite, and what a receiver takes is
    /// still bounded by what was sent.
    #[test]
    fn at_zero_distance_flux_is_infinite_and_the_fraction_is_one() {
        assert_eq!(flux_w_m2(1.0e20, 0.1, 0.0), f64::INFINITY);
        assert_eq!(flux_w_m2(0.0, 0.1, 0.0), 0.0);
        assert_eq!(received_fraction(0.1, 1.0, 0.0), 1.0);
        assert_eq!(received_fraction(0.1, 0.0, 0.0), 0.0);
        assert_eq!(distance_at_flux_m(0.0, 0.1, 1.0), 0.0);
    }

    /// Coming about with nothing lit, then pushed along its thrust to `tanh(α τ)`, then drifting
    /// at that: continuous at both edges, and the rapidity is the proper acceleration times the
    /// proper time lit.
    #[test]
    fn a_boost_turns_then_pushes_along_its_thrust_and_drifts_after() {
        let thrust = DVec3::new(0.6, -0.8, 0.0);
        let beta0 = DVec3::new(0.0, 0.0, 1.0e-3);
        let boost = Boost::plan(DVec3::ZERO, beta0, 100.0, thrust, -thrust, 5.0, 3_600.0, DVec3::X, 0.01);
        assert!(boost.turn_s() > 0.0 && boost.lights_s() == 100.0 + boost.turn_s());
        assert_eq!(boost.thrust_at(boost.lights_s() - 1.0e-3), DVec3::ZERO);
        assert_eq!(boost.thrust_at(boost.lights_s()), thrust);
        assert_eq!(boost.thrust_at(boost.out_s()), DVec3::ZERO);
        for edge in [boost.lights_s(), boost.out_s()] {
            let (before, after) = (boost.state_at(edge - 1.0e-6).0, boost.state_at(edge + 1.0e-6).0);
            assert!((before - after).length() * JULIAN_YEAR_S < 1.0e-5, "a jump at {edge}");
        }
        let (_, end) = boost.state_at(boost.out_s());
        let (_, later) = boost.state_at(boost.out_s() + 1.0e4);
        assert_eq!(end, later, "still pushed after it went out");
        let gained = crate::boost::velocity_to_frame(end, beta0);
        assert!((gained.normalize() - thrust).length() < 1.0e-9, "{gained}");
        assert!((gained.length().atanh() - boost.planned_rapidity()).abs() < 1.0e-9 * boost.planned_rapidity());
        let alpha = 5.0 * G0 / C_M_S;
        assert!((boost.planned_rapidity() / alpha - boost.proper_s(boost.out_s()) + boost.turn_s() / gamma_of(beta0)).abs() < 1.0e-6);
    }

    /// Through the wire's recipe and back, it lights and goes out at the same instants and is in
    /// the same place.
    #[test]
    fn a_boost_comes_back_from_its_recipe_unchanged() {
        let mut state = crate::motion::ShipState::at(DVec3::new(1.0e-6, 0.0, 0.0));
        state.begin_boosting(Boost::plan(state.position_ly, DVec3::ZERO, 10.0, DVec3::Y, -DVec3::Y, 0.5, 600.0, DVec3::X, 0.01));
        let wire = lc_proto::Motion::from(&state.snapshot());
        let back = crate::resume::Snapshot::from(&wire).restore(None, 10.0);
        assert_eq!(back.motive, state.motive);
    }

    /// A lit emission commits all it will draw, spends it as it goes, and putting it out early
    /// releases the rest.
    #[test]
    fn a_lit_emission_is_committed_and_released_when_put_out() {
        use crate::fitting::Lit;
        let motion = crate::motion::ShipState::at(DVec3::ZERO);
        let mut fitting = Fitting::full(Form::starting(), Balance::DEFAULT, 0.0);
        fitting.light(Lit { from_s: 0.0, until_s: 100.0, power_w: 1.0e19 });
        assert_eq!(fitting.committed_j_at(&motion, 0.0), 1.0e21);
        assert!((fitting.committed_j_at(&motion, 40.0) - 6.0e20).abs() < 1.0e6);
        fitting.settle(&motion, 40.0);
        fitting.darken();
        assert_eq!(fitting.committed_j_at(&motion, 40.0), 0.0);
        assert!(fitting.lit().is_empty());
    }

    /// A receiver whose shadow covers the spot takes everything; past that it takes its share.
    #[test]
    fn the_fraction_saturates_where_the_spot_shrinks_to_the_shadow() {
        let half = 0.01;
        let shadow = 100.0;
        let edge = (shadow / cone_solid_angle_sr(half)).sqrt();
        assert_eq!(received_fraction(half, shadow, 0.999 * edge), 1.0);
        let past = received_fraction(half, shadow, 2.0 * edge);
        assert!((past - 0.25).abs() < 1.0e-12, "{past}");
    }

    /// A crossing's drive sends `F c` at the mass the ship has as it burns, along the exhaust, at
    /// `drive_spread_rad`, and nothing while it coasts or flips. The same crossing at no more than
    /// `rcs_accel_g` is on the thrusters, at `rcs_spread_rad`.
    #[test]
    fn a_lit_drive_sends_f_c_along_its_exhaust() {
        use crate::craft::{Craft, CraftId, Kind};
        use crate::flight::{Drive, STANDOFF_LY};
        use crate::motion::{Change, Event, ShipId};
        let b = Balance::DEFAULT;
        for (accel_g, jet, spread) in [(5.0, Jet::Drive, b.drive_spread_rad), (b.rcs_accel_g, Jet::Thrusters, b.rcs_spread_rad)] {
            let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
            craft.fit(Some(Fitting::full(Form::starting(), b, 0.0)));
            let drive = Drive { accel_g, ..craft.turning(craft.kind.drive()) };
            let to_ly = DVec3::X * (STANDOFF_LY + 1.0e-6);
            craft.apply(&Event { ship: ShipId(1), at_t: 0.0, change: Change::Cross { to_ly, drive } }).unwrap();
            let crate::motion::Motive::Crossing(cruise) = &craft.motion.motive else { panic!() };
            let [lit, _, flip, brake, _] = cruise.phase_changes_s();
            let burning_s = 0.5 * (lit + flip);
            let [out] = exhaust(&craft, &b, burning_s)[..] else { panic!("one jet") };
            assert_eq!((out.jet, out.half_angle_rad), (jet, spread));
            assert!(out.axis.angle_between(-DVec3::X) < 1.0e-9, "{}", out.axis);
            let want_w = Drive::exhaust_w(craft.mass_kg_at(burning_s), accel_g);
            assert!((out.power_w / want_w - 1.0).abs() < 1.0e-12, "{} {want_w}", out.power_w);
            assert!(exhaust(&craft, &b, 0.5 * (flip + brake)).is_empty(), "lit through the flip");
        }
    }
}
