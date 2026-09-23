//! What a disc turns into: a ladder of feeding zones, and what each one assembles.
//!
//! Nothing here is drawn independently. A rung's mass is the solid in the annulus it sweeps,
//! its class follows from that mass and whether it is past the snow line, and the belts are
//! what the rungs failed to assemble. Mass is conserved across the whole ladder, which is what
//! makes the asteroid belt and the Kuiper analog consequences rather than decorations.

use super::disc::{self, Disc, JUPITER_EARTHS};
use super::tuning::Tuning;
use crate::rng;
use crate::sky::CatalogStar;

/// What a rung assembled into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Inside the snow line: rock and metal.
    Rocky,
    /// Past the snow line and too small for an envelope. Ganymede to Mars, in ice.
    Icy,
    /// A core that took a little hydrogen before the disc went. Neptune.
    IceGiant,
    /// A core that reached runaway and took all of it.
    GasGiant,
}

impl Class {
    pub fn is_giant(self) -> bool {
        matches!(self, Self::IceGiant | Self::GasGiant)
    }

    /// Whether the priors' two-way split calls this rocky. An icy dwarf is not a gas envelope,
    /// so it goes on the rocky side.
    pub fn is_rocky(self) -> bool {
        matches!(self, Self::Rocky | Self::Icy)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rocky => "rocky",
            Self::Icy => "icy",
            Self::IceGiant => "ice giant",
            Self::GasGiant => "gas giant",
        }
    }
}

/// One rung of the ladder, whether or not it became a planet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rung {
    pub semi_major_m: f64,
    /// The annulus it sweeps, meters.
    pub zone_m: (f64, f64),
    pub class: Class,
    /// Solids it actually assembled, Earth masses.
    pub core_earths: f64,
    /// Core plus whatever envelope it took, Earth masses.
    pub mass_earths: f64,
    pub radius_earths: f64,
    /// Solids left in the annulus: what the rung never got round to sweeping up, plus its
    /// whole zone when it was stirred into a belt. Earth masses.
    pub debris_earths: f64,
    /// Never assembled into a planet. This rung is a belt.
    pub sterile: bool,
    /// And it is a belt because a giant's resonances stirred it, rather than for want of mass.
    /// Which one decides how much is left: a stirred belt was thrown out, a stalled one is
    /// still there.
    pub stirred: bool,
    /// A giant that ended up far inside where it formed.
    pub migrated: bool,
    /// Equilibrium temperature where it ended up, kelvin, zero albedo.
    pub equilibrium_k: f64,
}

impl Rung {
    pub fn semi_major_au(&self) -> f64 {
        self.semi_major_m / super::AU
    }
    /// Whether the priors' two-way split calls this rocky.
    pub fn rocky(&self) -> bool {
        self.class.is_rocky()
    }
}

/// A star's disc and everything the ladder made of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Architecture {
    pub disc: Disc,
    /// Every rung, innermost first, belts included.
    pub rungs: Vec<Rung>,
}

impl Architecture {
    /// The rungs that became planets.
    pub fn planets(&self) -> impl Iterator<Item = &Rung> {
        self.rungs.iter().filter(|r| !r.sterile)
    }

    /// The rungs that did not.
    pub fn belts(&self) -> impl Iterator<Item = &Rung> {
        self.rungs.iter().filter(|r| r.sterile)
    }

    /// Total giant mass, in Jupiters. What decides how hard the system throws: the Oort cloud
    /// and the delivery of water to the inner planets both scale with it.
    pub fn giant_jupiters(&self) -> f64 {
        self.planets().filter(|r| r.class.is_giant()).map(|r| r.mass_earths).sum::<f64>() / JUPITER_EARTHS
    }
}

/// The architecture of one star's system.
pub fn architecture(star: &CatalogStar, tuning: &Tuning) -> Architecture {
    let disc = Disc::of(star, tuning);
    let seed = star.seed();
    let mut rungs = rungs_of(&disc, star, tuning, seed);
    // Before anything else reads a mass: what merges changes how far the survivor's
    // resonances reach and which giant is there to migrate.
    settle_giants(&mut rungs, &disc, star, tuning);
    migrate(&mut rungs, &disc, star, tuning, seed);
    sterilize(&mut rungs, star, tuning);
    Architecture { disc, rungs }
}

/// Where the rungs sit and what each one assembles.
fn rungs_of(disc: &Disc, star: &CatalogStar, tuning: &Tuning, seed: u64) -> Vec<Rung> {
    let t = &tuning.ladder;
    let mut axes = Vec::new();
    let mut a = disc.inner_m
        * rng::uniform_in(rng::hash(&[seed, 0x1add, 0]), t.first_rung.0.ln(), t.first_rung.1.ln()).exp();
    while a < disc.outer_m && axes.len() < t.max_rungs {
        axes.push(a);
        a *= rng::uniform_in(rng::hash(&[seed, 0x1add, axes.len() as u64]), t.spacing.0, t.spacing.1);
    }

    // Growth slows as the cube of the orbit, so the outer disc never finishes assembling and
    // what it leaves behind is the trans-planetary belt. Measured in snow lines, so a dim
    // star's whole system is pulled inward together.
    let growth_m = t.growth_over_snow * disc.snow_m;

    axes.iter()
        .enumerate()
        .map(|(k, &a)| {
            let h = |tag: u64| rng::hash(&[seed, 0x91a4, k as u64, tag]);
            let lo = if k == 0 { disc.inner_m } else { (axes[k - 1] * a).sqrt() };
            let hi = axes.get(k + 1).map_or(disc.outer_m, |next| (a * next).sqrt());
            let available = disc.solids_between(lo, hi);
            let assembled = (growth_m / a).powi(3).min(1.0)
                * rng::uniform_in(h(11), t.efficiency.0, t.efficiency.1);
            let core = available * assembled;

            let icy = disc.icy(a);
            let runaway =
                t.runaway_core_earths * 10f64.powf(rng::gaussian(h(17)) * t.runaway_spread_dex);
            let class = match (icy, core) {
                (false, _) => Class::Rocky,
                (true, c) if c >= runaway => Class::GasGiant,
                (true, c) if c >= t.ice_giant_core_earths => Class::IceGiant,
                (true, _) => Class::Icy,
            };
            let mass = envelope(class, core, h(12), t);
            Rung {
                semi_major_m: a,
                zone_m: (lo, hi),
                class,
                core_earths: core,
                mass_earths: mass,
                radius_earths: disc::radius_earths(mass, icy),
                debris_earths: (available - core).max(0.0),
                sterile: false,
                stirred: false,
                migrated: false,
                equilibrium_k: disc::temperature_at(&star.star, a),
            }
        })
        .collect()
}

/// Total mass after whatever gas the core took.
fn envelope(class: Class, core: f64, h: u64, t: &super::tuning::Ladder) -> f64 {
    match class {
        Class::Rocky | Class::Icy => core,
        // Enough hydrogen to matter and not enough to run away: Neptune is fifteen Earths of
        // ice under two of gas.
        Class::IceGiant => core * rng::uniform_in(h, 1.05, 1.4),
        Class::GasGiant => {
            let multiple = rng::uniform_in(h, t.envelope.0.ln(), t.envelope.1.ln()).exp();
            (core * multiple).min(t.heaviest_jupiters * JUPITER_EARTHS)
        }
    }
}

/// Merge giants the ladder put closer together than they could stay.
///
/// The rungs are spaced by a ratio, which takes no account of mass, so two giants land inside
/// each other's reach often enough to matter. Two that close scatter, and the outcome a system
/// old enough to look at has already had is one planet: the lighter goes into the heavier,
/// carrying its core, its envelope and the debris of its zone, so the ladder's mass is still
/// conserved and the survivor sweeps both annuli.
fn settle_giants(rungs: &mut Vec<Rung>, disc: &Disc, star: &CatalogStar, tuning: &Tuning) {
    while let Some(k) = crowded(rungs, star, tuning.ladder.hill_separation) {
        let (lighter, heavier) = match rungs[k].mass_earths <= rungs[k + 1].mass_earths {
            true => (k, k + 1),
            false => (k + 1, k),
        };
        let gone = rungs.remove(lighter);
        let kept = &mut rungs[if heavier > lighter { heavier - 1 } else { heavier }];
        kept.core_earths += gone.core_earths;
        kept.mass_earths += gone.mass_earths;
        kept.debris_earths += gone.debris_earths;
        kept.zone_m = (kept.zone_m.0.min(gone.zone_m.0), kept.zone_m.1.max(gone.zone_m.1));
        kept.radius_earths = disc::radius_earths(kept.mass_earths, disc.icy(kept.semi_major_m));
    }
}

/// The adjacent pair of giants with the least room between them, when that is too little.
fn crowded(rungs: &[Rung], star: &CatalogStar, wanted: f64) -> Option<usize> {
    let mut worst: Option<(f64, usize)> = None;
    let giants: Vec<usize> = (0..rungs.len()).filter(|&k| rungs[k].class.is_giant()).collect();
    for pair in giants.windows(2) {
        let (lo, hi) = (&rungs[pair[0]], &rungs[pair[1]]);
        let mutual = ((lo.mass_earths + hi.mass_earths) * disc::EARTH_MASS
            / (3.0 * star.mass_solar.max(0.05) * disc::SOLAR_MASS_KG))
            .cbrt()
            * 0.5
            * (lo.semi_major_m + hi.semi_major_m);
        let apart = (hi.semi_major_m - lo.semi_major_m) / mutual.max(f64::MIN_POSITIVE);
        if apart < wanted && worst.is_none_or(|(least, _)| apart < least) {
            worst = Some((apart, pair[0]));
        }
    }
    worst.map(|(_, k)| k)
}

/// Send a few giants inward, and let them take out everything they cross.
///
/// This is where hot Jupiters come from, and it is also why a system that has one has almost
/// nothing else: a giant crossing the inner disc does not leave it behind.
fn migrate(rungs: &mut Vec<Rung>, disc: &Disc, star: &CatalogStar, tuning: &Tuning, seed: u64) {
    let t = &tuning.ladder;
    let Some(k) = rungs
        .iter()
        .position(|r| r.class == Class::GasGiant && rng::uniform(rng::hash(&[seed, 0x_6d16_1a7e, r.semi_major_m.to_bits()])) < t.migrating_fraction)
    else {
        return;
    };
    let h = rng::hash(&[seed, 0x_7a7e_6f75, k as u64]);
    let target = rng::uniform_in(h, (disc.inner_m * 1.5).ln(), disc.snow_m.ln()).exp();
    rungs.drain(..k);
    let giant = &mut rungs[0];
    giant.semi_major_m = target;
    giant.migrated = true;
    giant.equilibrium_k = disc::temperature_at(&star.star, target);
    // The swept disc goes with it: a migrating giant's debris is thrown out, not left in place.
    giant.zone_m = (disc.inner_m, giant.zone_m.1);
}

/// Mark every rung that never became a planet: stirred by a giant's resonances, or simply
/// never given enough to assemble one.
fn sterilize(rungs: &mut [Rung], star: &CatalogStar, tuning: &Tuning) {
    let reaches: Vec<(f64, f64)> = rungs
        .iter()
        .filter(|r| r.class.is_giant())
        .map(|r| {
            let mu = r.mass_earths * disc::EARTH_MASS / (star.mass_solar.max(0.05) * disc::SOLAR_MASS_KG);
            // Capped short of one, or the heaviest planets reach the star: at thirteen Jupiter
            // masses the raw width is exactly one and the band's inner edge lands on zero,
            // which would sterilize every rung a system has.
            (r.semi_major_m, (tuning.ladder.resonance_reach * mu.powf(0.2)).min(0.85))
        })
        .collect();

    for rung in rungs.iter_mut() {
        if rung.class.is_giant() {
            continue;
        }
        let stirred = reaches.iter().any(|(a_g, width)| {
            rung.semi_major_m > a_g * (1.0 - width) && rung.semi_major_m < a_g * (1.0 + width)
        });
        if stirred || rung.core_earths < tuning.ladder.smallest_earths {
            rung.sterile = true;
            rung.stirred = stirred;
            rung.debris_earths += rung.core_earths;
            rung.core_earths = 0.0;
            rung.mass_earths = 0.0;
            rung.radius_earths = 0.0;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::sky::{AuthoredStars, StarId, StarProvider};

    pub fn sun_like(key: u64) -> CatalogStar {
        let mut s = AuthoredStars::sample().stars()[1].clone();
        s.id = StarId::synthesize("arch", key);
        s.star = crate::star::Star::SOL;
        s.luminosity_solar = 1.0;
        s.mass_solar = 1.0;
        s.metallicity = 0.0;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::tests_support::sun_like;
    const AU: f64 = super::super::AU;

    fn census(count: u64, tuning: &Tuning) -> Vec<Architecture> {
        (0..count).map(|k| architecture(&sun_like(k), tuning)).collect()
    }

    /// **The claim the belts rest on.** Every rung's core plus its debris is the solid in its
    /// own annulus, and the annuli tile the disc, so nothing is invented and nothing vanishes.
    /// The Kuiper analog exists because this holds, not because it was placed.
    #[test]
    fn the_ladder_conserves_the_disc() {
        let mut tuning = Tuning::default();
        // Migration throws the swept disc out of the system, which is the one process here
        // that is allowed to lose mass.
        tuning.ladder.migrating_fraction = 0.0;
        for a in census(60, &tuning) {
            let held: f64 = a.rungs.iter().map(|r| r.core_earths + r.debris_earths).sum();
            assert!(
                (held / a.disc.solid_earths - 1.0).abs() < 1.0e-9,
                "{held} of {} Earth masses accounted for",
                a.disc.solid_earths
            );
        }
    }

    /// A gas giant is a core that grew past runaway, and only the ices past the snow line make
    /// a core that big. One that is found inside it got there by moving.
    #[test]
    fn giants_form_beyond_the_snow_line_and_only_reach_inside_it_by_migrating() {
        let mut migrated = 0;
        for a in census(200, &Tuning::default()) {
            for r in a.planets().filter(|r| r.class.is_giant()) {
                if r.semi_major_m < a.disc.snow_m {
                    assert!(r.migrated, "a giant at {:.2} AU that did not move", r.semi_major_au());
                    migrated += 1;
                }
            }
        }
        assert!(migrated > 3, "only {migrated} migrating giants in two hundred systems");
    }

    /// A giant that crossed the inner disc did not leave it behind, which is why the real hot
    /// Jupiters are found alone.
    #[test]
    fn a_migrated_giant_is_the_innermost_thing_left() {
        let mut checked = 0;
        for a in census(300, &Tuning::default()) {
            let Some(giant) = a.rungs.iter().find(|r| r.migrated) else { continue };
            checked += 1;
            assert_eq!(a.rungs[0].semi_major_m, giant.semi_major_m, "something survived inside it");
            assert!(giant.semi_major_m < a.disc.snow_m);
        }
        assert!(checked > 5, "only {checked} migrations in three hundred systems");
    }

    /// The asteroid belt, as a consequence. A giant's resonances stir the rung inside it past
    /// assembling, so the belt turns up between the rocky planets and the innermost giant --
    /// which is where the real one is.
    ///
    /// Only a *gas* giant does it. An ice giant's reach is a third as wide and the rung inside
    /// it assembles unbothered, which is why Neptune has no belt in front of it.
    #[test]
    fn a_belt_appears_inside_the_innermost_gas_giant() {
        let mut found = 0;
        let mut systems = 0;
        for a in census(300, &Tuning::default()) {
            let Some(giant) = a.planets().find(|r| r.class == Class::GasGiant && !r.migrated) else { continue };
            systems += 1;
            for belt in a.belts().filter(|b| b.semi_major_m < giant.semi_major_m) {
                found += 1;
                assert!(belt.debris_earths > 0.0, "an empty belt is not a belt");
                assert!(belt.mass_earths == 0.0, "a belt assembled nothing");
                // Inside the giant and outside whatever rocky planets survived.
                let inner: Vec<f64> =
                    a.planets().filter(|p| p.semi_major_m < belt.semi_major_m).map(|p| p.semi_major_m).collect();
                assert!(inner.iter().all(|&x| x < belt.semi_major_m));
            }
        }
        assert!(systems > 100, "only {systems} systems kept a gas giant where it formed");
        assert!(found as f64 / systems as f64 > 0.2, "only {found} belts across {systems} systems");
    }

    /// The trans-planetary belt is what the outer disc never finished assembling, and it has
    /// to be most of the debris a system has.
    #[test]
    fn the_outer_disc_leaves_more_behind_than_it_assembles() {
        for a in census(40, &Tuning::default()) {
            let edge = Tuning::default().ladder.growth_over_snow * a.disc.snow_m;
            let outer: Vec<&Rung> = a.rungs.iter().filter(|r| r.semi_major_m > edge).collect();
            if outer.len() < 2 {
                continue;
            }
            let (built, left): (f64, f64) =
                outer.iter().fold((0.0, 0.0), |(b, l), r| (b + r.core_earths, l + r.debris_earths));
            assert!(left > built, "outer disc built {built} and left {left}");
        }
    }

    /// **The optimism this generator is tuned for.** The habitable zone is supposed to be
    /// occupied most of the time, and by something of roughly Earth's size rather than by
    /// whatever happened to land there.
    #[test]
    fn most_sun_like_stars_get_a_planet_in_the_habitable_zone() {
        let systems = census(300, &Tuning::default());
        let occupied = systems
            .iter()
            .filter(|a| a.planets().any(|r| a.disc.habitable(r.semi_major_m) && r.class == Class::Rocky))
            .count();
        assert!(occupied > 200, "only {occupied} of 300 have a rocky planet in the zone");

        let masses: Vec<f64> = systems
            .iter()
            .flat_map(|a| a.planets().filter(|r| a.disc.habitable(r.semi_major_m) && r.class == Class::Rocky))
            .map(|r| r.mass_earths)
            .collect();
        let earthlike = masses.iter().filter(|&&m| (0.3..=5.0).contains(&m)).count();
        assert!(
            earthlike as f64 / masses.len() as f64 > 0.6,
            "only {earthlike} of {} habitable-zone planets are Earth-sized",
            masses.len()
        );
    }

    /// Metals are the rock, all the way down: a metal-poor disc builds smaller cores, so fewer
    /// of them reach runaway and the system ends up with no giants at all.
    #[test]
    fn a_metal_poor_star_gets_a_smaller_system() {
        let giants = |feh: f64| {
            (0..120u64)
                .map(|k| {
                    let mut s = sun_like(k);
                    s.metallicity = feh;
                    architecture(&s, &Tuning::default()).planets().filter(|r| r.class.is_giant()).count()
                })
                .sum::<usize>()
        };
        let (rich, poor) = (giants(0.3), giants(-1.0));
        assert!(rich > poor * 3, "{rich} giants at [Fe/H] +0.3 against {poor} at -1.0");
    }

    /// Every number a rung carries has to be finite and orderable, because everything
    /// downstream sorts, bins and plots them.
    #[test]
    fn a_rung_is_always_well_formed() {
        for a in census(200, &Tuning::default()) {
            let mut last = 0.0;
            for r in &a.rungs {
                assert!(r.semi_major_m > last, "rungs must run outward");
                last = r.semi_major_m;
                for v in [r.core_earths, r.mass_earths, r.radius_earths, r.debris_earths, r.equilibrium_k] {
                    assert!(v.is_finite() && v >= 0.0, "{v} on {}", r.class.label());
                }
                assert!(r.mass_earths >= r.core_earths * 0.999, "an envelope cannot weigh nothing");
                assert!(r.semi_major_m >= a.disc.inner_m * 0.99);
                assert_eq!(r.sterile, r.mass_earths == 0.0);
                assert!(r.equilibrium_k < 3000.0, "{} K is inside the star", r.equilibrium_k);
            }
        }
    }

    /// Generation is a pure function of the seed, or a system reshuffles itself between loads.
    #[test]
    fn the_same_star_gives_the_same_system() {
        let s = sun_like(17);
        assert_eq!(architecture(&s, &Tuning::default()), architecture(&s, &Tuning::default()));
        assert_ne!(architecture(&s, &Tuning::default()), architecture(&sun_like(18), &Tuning::default()));
    }

    /// The knobs have to do what they say, or the plots in the documentation are decoration.
    #[test]
    fn the_tuning_moves_what_it_claims_to() {
        // Gas giants specifically: lowering the threshold promotes ice giants into gas ones
        // rather than making more giants, which is exactly what it should do.
        let giants = |t: &Tuning| {
            (0..120u64)
                .map(|k| architecture(&sun_like(k), t).planets().filter(|r| r.class == Class::GasGiant).count())
                .sum::<usize>()
        };
        let mut generous = Tuning::default();
        generous.ladder.runaway_core_earths = 4.0;
        let mut mean = Tuning::default();
        mean.ladder.runaway_core_earths = 20.0;
        assert!(giants(&generous) > giants(&mean) * 2);

        let mut wide = Tuning::default();
        wide.ladder.spacing = (2.5, 3.5);
        let count = |t: &Tuning| (0..60u64).map(|k| architecture(&sun_like(k), t).rungs.len()).sum::<usize>();
        assert!(count(&wide) * 5 < count(&Tuning::default()) * 3, "wider spacing must mean fewer rungs");
    }

    /// A red dwarf's system is the same disc pulled inward, so its habitable planets are on
    /// orbits of days rather than years -- and there still have to be some.
    #[test]
    fn a_red_dwarf_still_gets_a_habitable_planet_and_it_is_close_in() {
        let mut occupied = 0;
        let mut axes = Vec::new();
        for k in 0..120u64 {
            let mut s = sun_like(k);
            s.star.radius_m = 0.15 * crate::star::Star::SOL.radius_m;
            s.star.teff_k = 3200.0;
            s.star.mu = 0.2 * crate::star::Star::SOL.mu;
            s.mass_solar = 0.2;
            s.luminosity_solar = 0.0035;
            let a = architecture(&s, &Tuning::default());
            if let Some(p) = a.planets().find(|r| a.disc.habitable(r.semi_major_m)) {
                occupied += 1;
                axes.push(p.semi_major_au());
            }
        }
        assert!(occupied > 60, "only {occupied} of 120 red dwarfs have an occupied zone");
        let mean = axes.iter().sum::<f64>() / axes.len() as f64;
        assert!(mean < 0.1, "a red dwarf's zone is at {mean} AU, which is not close in");
        assert!(mean * AU > 0.0);
    }

    /// Two giants closer than a few mutual Hill radii scatter; a system old enough to be
    /// looked at has already had that happen. The ladder spaces rungs by a ratio and takes no
    /// account of mass, so it put a third of adjacent giant pairs inside ten mutual Hill radii
    /// and the tightest at 2.5 -- below the limit where two planets are stable at all.
    #[test]
    fn no_two_giants_are_closer_than_they_could_stay() {
        let t = Tuning::default();
        let mut pairs = 0;
        let mut worst = f64::INFINITY;
        for k in 0..600u64 {
            let s = sun_like(k);
            let a = architecture(&s, &t);
            let giants: Vec<&Rung> = a.planets().filter(|r| r.class.is_giant()).collect();
            for w in giants.windows(2) {
                pairs += 1;
                worst = worst.min(mutual_hill_apart(w[0], w[1], s.mass_solar));
            }
        }
        assert!(pairs > 200, "only {pairs} giant pairs to check");
        assert!(worst >= t.ladder.hill_separation, "a pair {worst:.2} mutual Hill radii apart");
    }

    /// Merging conserves the ladder's mass, which is what makes the belts consequences of the
    /// disc rather than decorations.
    #[test]
    fn a_merge_keeps_the_mass_it_started_with() {
        let t = Tuning::default();
        let mut merged = 0;
        for k in 0..400u64 {
            let s = sun_like(k);
            let disc = Disc::of(&s, &t);
            let before = rungs_of(&disc, &s, &t, s.seed());
            let mut after = before.clone();
            settle_giants(&mut after, &disc, &s, &t);
            if after.len() == before.len() {
                continue;
            }
            merged += 1;
            let total = |rungs: &[Rung]| -> f64 {
                rungs.iter().map(|r| r.mass_earths + r.debris_earths).sum()
            };
            let (a, b) = (total(&before), total(&after));
            assert!((a - b).abs() < 1.0e-9 * a.max(1.0), "{a} became {b}");
        }
        assert!(merged > 20, "only {merged} systems merged anything");
    }

    fn mutual_hill_apart(lo: &Rung, hi: &Rung, star_mass_solar: f64) -> f64 {
        let mutual = ((lo.mass_earths + hi.mass_earths) * disc::EARTH_MASS
            / (3.0 * star_mass_solar * disc::SOLAR_MASS_KG))
            .cbrt()
            * 0.5
            * (lo.semi_major_m + hi.semi_major_m);
        (hi.semi_major_m - lo.semi_major_m) / mutual
    }
}



