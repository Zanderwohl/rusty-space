//! Running the telescope: what it is committed to, and what that turns into on the record.
//!
//! Split out of [`crate::session`] because it is a different job from flying a ship: every
//! function here ends in something filed in a [`Knowledge`], and the rules it follows are
//! `lightcone/docs/22-provenance.md` rather than any of the flight documents.
//!
//! [`Knowledge`]: lc_world::knowledge::Knowledge

use std::mem;

use em_spectra::Band;
use lc_world::knowledge::survey::{self, Duty, Optics, Source, Sweep};
use lc_world::knowledge::{Bearing, Claim, Distance, Hop, NameKind, Naming, Sighting, Witness};
use lc_world::rng;
use lc_world::sky::StarId;

use crate::session::{CHART_ERROR, M_PER_LY, Session};

/// Who the charts a ship launches with came from. Not this ship, and not any craft it will
/// ever meet: a name for "somebody else measured this and we are taking their word".
pub const CHARTS: Witness = Witness(u64::MAX);

impl Session {
    /// What the ship looks through, and how far apart its elements are.
    ///
    /// One hull, so the baseline is the mirror. A swarm of telescopes flying in formation is
    /// [`Optics::joined`], and the difference is what it can see next to something bright.
    pub fn optics(&self) -> Optics {
        Optics::of(self.telescope)
    }

    /// Every star as a point source arriving here, in the band the survey works in.
    ///
    /// Order matches [`Session::stars`], because a detection is indexed into this and the
    /// glare test needs the whole sky to compare against.
    fn sky_sources(&mut self, band: Band) -> Vec<Source> {
        let stars = mem::take(&mut self.stars);
        let here = self.ship.motion.position_ly;
        let sources = stars
            .iter()
            .map(|star| {
                let luminosity = *self.luminosity.entry(star.id).or_insert_with(|| {
                    let unit = survey::flux_from(&star.star, band, M_PER_LY);
                    4.0 * std::f64::consts::PI * M_PER_LY * M_PER_LY * unit
                });
                let offset = star.position_ly - here;
                let distance_m = offset.length() * M_PER_LY;
                let flux = if distance_m > 0.0 {
                    luminosity / (4.0 * std::f64::consts::PI * distance_m * distance_m)
                } else {
                    0.0
                };
                Source {
                    star: star.id,
                    toward: offset.normalize_or_zero(),
                    flux_w_m2: flux,
                }
            })
            .collect();
        self.stars = stars;
        sources
    }

    /// Run whatever the telescope is committed to, for however much coordinate time has passed.
    ///
    /// Exposure is elapsed coordinate time, not a number typed into a panel: a measurement
    /// labelled with an integration it did not get is a lie about its own error bars. At the
    /// design rate a rendered frame is a couple of minutes of it, so a sample lands whenever
    /// the integration is actually complete.
    pub fn tick_instruments(&mut self, integration_s: f64) {
        let now = self.coordinate_time_s();
        match self.duty.clone() {
            Duty::Idle => {}
            Duty::Stare(id) => {
                let elapsed = now - self.sampled_s;
                if elapsed >= integration_s.max(1.0) {
                    self.aim(Some(id));
                    self.observe(elapsed);
                    self.fix_position(id, elapsed, now);
                    self.sampled_s = now;
                }
            }
            duty @ Duty::Watch { .. } => {
                let Duty::Watch { dwell_s, .. } = duty else {
                    return;
                };
                let Some(slot) = duty.slot_at(now) else {
                    return;
                };
                let previous = std::mem::replace(&mut self.slot, slot);
                // The turn that just ended is the only one with a full dwell behind it, and
                // on the first tick there is no such turn.
                if previous != slot
                    && previous != i64::MIN
                    && let Some(id) = duty.target_at(now - dwell_s.max(1.0))
                {
                    self.aim(Some(id));
                    self.observe(dwell_s);
                    self.fix_position(id, dwell_s, now);
                }
                self.aim(duty.target_at(now));
            }
            Duty::Sweep(sweep) => {
                self.sweep(&sweep, self.swept_s, now);
                self.swept_s = now;
            }
        }
    }

    /// File a bearing to a star the telescope is already pointed at.
    ///
    /// A stare measures where something is as well as how bright it is, which is why watching
    /// one star from a moving ship eventually gives its distance without any survey at all.
    fn fix_position(&mut self, id: StarId, exposure_s: f64, now_s: f64) {
        let Some(band) = self.optics().band() else {
            return;
        };
        let sky = self.sky_sources(band);
        let Some(index) = sky.iter().position(|s| s.star == id) else {
            return;
        };
        let optics = self.optics();
        let seen = survey::look(
            &optics,
            &sky,
            index,
            exposure_s,
            self.ship.motion.position_ly,
            now_s,
            self.knowledge.owner,
        );
        if let Some(seen) = seen {
            self.knowledge.sighted(id, seen);
        }
    }

    /// Collect whatever fields the sweep finished between two coordinate times.
    fn sweep(&mut self, sweep: &Sweep, from_s: f64, to_s: f64) {
        let Some(band) = self.optics().band() else {
            return;
        };
        let plan = sweep.plan();
        let here = self.ship.motion.position_ly;
        let due: Vec<(usize, f64)> = self
            .stars
            .iter()
            .enumerate()
            .filter_map(|(i, star)| {
                let toward = (star.position_ly - here).normalize_or_zero();
                plan.observed_between(toward, from_s, to_s)
                    .map(|at| (i, at))
            })
            .collect();
        if due.is_empty() {
            return;
        }
        let sky = self.sky_sources(band);
        let optics = self.optics();
        let witness = self.knowledge.owner;
        for (index, at) in due {
            if let Some(seen) =
                survey::look(&optics, &sky, index, sweep.exposure_s(), here, at, witness)
            {
                self.knowledge.sighted(sky[index].star, seen);
            }
        }
    }

    /// Issue the charts a ship leaves port with.
    ///
    /// A craft does not start from nothing — it starts from somebody else's parallax
    /// programme, which is exactly as good as whoever ran it and does not extend past where
    /// they were looking. So the nearby sky arrives as [`Claim`]s from a charting office the
    /// ship has never met, held on that office's word until the ship measures one for itself,
    /// and everything beyond `reach_ly` is sky nobody aboard has ever detected.
    ///
    /// [`Claim`]: lc_world::knowledge::Claim
    pub fn issue_charts(&mut self, reach_ly: f64) {
        let Some(band) = self.optics().band() else {
            return;
        };
        let now = self.coordinate_time_s();
        let from = self.ship.motion.position_ly;
        let hop = Hop {
            from: CHARTS,
            to: self.knowledge.owner,
            sent_s: now,
            received_s: now,
        };
        let sources = self.sky_sources(band);
        let stars = mem::take(&mut self.stars);
        for (star, source) in stars.iter().zip(sources.iter()) {
            let offset = star.position_ly - from;
            let distance = offset.length();
            if distance > reach_ly || distance <= 0.0 {
                continue;
            }
            // A catalogue distance is a parallax like any other, so its error grows with
            // range; a percent at the far edge of the charted volume is a generous office.
            let sigma_ly = CHART_ERROR * distance;
            let slip = rng::gaussian(rng::hash(&[star.id.get(), 0x0_c_4_a_7]));
            self.knowledge.sighted(
                star.id,
                Sighting {
                    witness: CHARTS,
                    observed_s: now,
                    bearing: Bearing {
                        observer_ly: from,
                        toward: source.toward,
                        sigma_rad: sigma_ly / distance,
                    },
                    band,
                    flux: source.flux_w_m2,
                    flux_sigma: source.flux_w_m2 * CHART_ERROR,
                    lineage: vec![hop],
                },
            );
            if let Some(name) = star.provenance.name.clone() {
                self.knowledge.named(
                    star.id,
                    Naming {
                        witness: CHARTS,
                        name,
                        kind: NameKind::Given,
                        stated_s: now,
                        lineage: vec![hop],
                    },
                );
            }
            self.knowledge.told(
                star.id,
                Claim {
                    witness: CHARTS,
                    distance: Distance::Measured {
                        position_ly: from + offset.normalize() * (distance + slip * sigma_ly),
                        sigma_ly,
                    },
                    stated_s: now,
                    lineage: vec![hop],
                },
            );
        }
        self.stars = stars;
    }
}
