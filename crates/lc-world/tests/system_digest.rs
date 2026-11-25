//! Default systems, pinned, and what the ocean-first setting does to them. See
//! `lightcone/docs/35-system-contents.md`.

use lc_world::sky::generate::{self, Tuning, ocean};
use lc_world::sky::{AuthoredStars, CatalogStar, StarId, StarProvider};

/// A thousand stars of three types across `[Fe/H]` −1.2 to +0.4.
fn stars() -> Vec<CatalogStar> {
    let base = AuthoredStars::sample().stars().to_vec();
    (0..1000u64)
        .map(|k| {
            let mut star = base[(k % 3) as usize].clone();
            star.id = StarId::synthesize("system-digest", k);
            star.metallicity = -1.2 + 1.6 * ((k * 37 % 100) as f64 / 100.0);
            star
        })
        .collect()
}

fn digest(tuning: &Tuning) -> u64 {
    let mut text = String::new();
    for star in stars() {
        let sys = generate::system_with(&star, tuning);
        for p in &sys.planets {
            text += &format!("{} {:e} {:e} {:e} {:e} {} {:e} {}\n", p.name, p.semi_major_m, p.mass_kg, p.radius_m, p.water_fraction, p.habitable, p.life, p.moons.len());
        }
        for pop in &sys.populations {
            // `kind` is left out so the digest still matches master's.
            let debug = format!("{pop:?}");
            let kind = format!("kind: {:?}, ", pop.kind);
            text += &debug.replacen(&kind, "", 1);
            text += "\n";
        }
    }
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// Computed on master before the ocean-first setting existed. A change here moved default systems.
#[test]
fn default_systems_are_unchanged() {
    assert_eq!(format!("{:016x}", digest(&Tuning::default())), "91ef98f6f95e4e3e");
}

fn ocean_worlds(sys: &generate::GeneratedSystem) -> usize {
    sys.planets.iter().filter(|p| p.habitable && p.water_fraction >= ocean::SEAS_WATER_FRACTION).count()
}

/// At 1, every eligible star has an ocean world; an ineligible one builds its default system.
#[test]
fn at_one_every_eligible_star_has_an_ocean_world() {
    let mut tuning = Tuning::default();
    tuning.world.ocean_first = 1.0;
    let (mut eligible, mut ineligible) = (0, 0);
    for star in stars() {
        let sys = generate::system_with(&star, &tuning);
        if ocean::eligible(&star, &tuning) {
            assert!(ocean_worlds(&sys) >= 1, "{} at [Fe/H] {}", star.id.get(), star.metallicity);
            assert!(sys.architecture.rungs.iter().any(|r| r.ocean && !r.sterile));
            eligible += 1;
        } else {
            assert_eq!(sys, generate::system_for(&star));
            ineligible += 1;
        }
    }
    assert!(eligible > 100 && ineligible > 50, "{eligible} eligible, {ineligible} not");
}

/// A star that loses the draw builds exactly its default system; one that wins has its ocean world.
#[test]
fn a_lost_draw_changes_nothing() {
    let mut tuning = Tuning::default();
    tuning.world.ocean_first = 0.5;
    let (mut same, mut changed) = (0, 0);
    for star in stars() {
        let sys = generate::system_with(&star, &tuning);
        if sys.architecture.rungs.iter().any(|r| r.ocean) {
            assert!(ocean_worlds(&sys) >= 1);
            changed += 1;
        } else {
            assert_eq!(sys, generate::system_for(&star));
            same += 1;
        }
    }
    assert!(same > 300 && changed > 100, "{same} unchanged, {changed} ocean-first");
}

/// An ocean-first ladder still conserves the disc: the ocean world's mass comes from its own
/// annulus. Migration is off, the one process allowed to lose mass.
#[test]
fn an_ocean_first_ladder_conserves_the_disc() {
    let mut tuning = Tuning::default();
    tuning.world.ocean_first = 1.0;
    tuning.ladder.migrating_fraction = 0.0;
    for star in stars().iter().filter(|s| ocean::eligible(s, &tuning)).take(200) {
        let a = generate::architecture(star, &tuning);
        let held: f64 = a.rungs.iter().map(|r| r.core_earths + r.debris_earths).sum();
        assert!((held / a.disc.solid_earths - 1.0).abs() < 1e-9, "{held} of {}", a.disc.solid_earths);
    }
}
