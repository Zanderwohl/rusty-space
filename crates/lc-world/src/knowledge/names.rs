//! The rules that name something before anybody chooses a name for it.
//!
//! Every rule is deterministic given what the assigning craft knew, and nothing a rule assigned
//! is ever reassigned. See "Planet letters" in `lightcone/docs/23-factions.md`.

use glam::DVec3;

use crate::flight::JULIAN_YEAR_S;

/// A designation from the direction something was found in, ecliptic degrees.
///
/// Fixed at discovery rather than recomputed, because the bearing drifts as the observer moves.
pub fn designation(toward: DVec3) -> String {
    let toward = toward.normalize_or(DVec3::X);
    let longitude = toward.y.atan2(toward.x).rem_euclid(std::f64::consts::TAU).to_degrees();
    let latitude = toward.z.clamp(-1.0, 1.0).asin().to_degrees();
    format!("{longitude:05.1}{latitude:+05.1}")
}

/// A small body's designation: the coordinate year it was found in, and its place in that
/// year's finds around the same star.
pub fn discovery_designation(discovered_s: f64, order: u32) -> String {
    let year = (discovered_s / JULIAN_YEAR_S).floor() as i64;
    format!("{year}-{order}")
}

/// Where a system's planets are expected to be, for stars up to a luminosity.
///
/// Slot `n` sits at `innermost_au_per_sqrt_l * sqrt(L) * ratio^n`: a given temperature moves out
/// with the square root of luminosity, and neighbors sit a roughly constant ratio apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacingRow {
    pub up_to_luminosity_solar: f64,
    pub innermost_au_per_sqrt_l: f64,
    pub ratio: f64,
}

/// The expected spacing, by luminosity. Must match `sky::generate`, or letters are left for
/// planets it never makes.
///
/// The generator's planets average 0.090 `sqrt(L)` AU for the innermost and a step of 1.77,
/// and the table is deliberately finer than both. A slot is chosen by rounding, so what
/// decides how often two planets want the same letter is the *spread* in the step and not its
/// mean: steps run from 1.35 to 1.85 and a grid at the mean collides on every short one. At
/// 1.55 the rate is four percent and the letters still stop well short of the alphabet.
pub const SPACING: &[SpacingRow] = &[SpacingRow {
    up_to_luminosity_solar: f64::INFINITY,
    innermost_au_per_sqrt_l: 0.060,
    ratio: 1.50,
}];

/// Which expected slot an orbit falls in, counting outward from zero.
pub fn expected_slot(semi_major_au: f64, luminosity_solar: f64, table: &[SpacingRow]) -> usize {
    let Some(row) = table
        .iter()
        .find(|r| luminosity_solar <= r.up_to_luminosity_solar)
        .or(table.last())
    else {
        return 0;
    };
    let innermost = row.innermost_au_per_sqrt_l * luminosity_solar.max(1e-4).sqrt();
    if !(semi_major_au > 0.0 && innermost > 0.0 && row.ratio > 1.0) {
        return 0;
    }
    ((semi_major_au / innermost).ln() / row.ratio.ln()).round().max(0.0) as usize
}

/// The letter a newly found planet takes.
///
/// `placed` is the letters this craft already uses for the same star, with their orbits.
/// Alphabetical order is orbital order, from `b` (`a` is the star). The expected slot's letter is
/// taken when it fits; otherwise a second letter after the neighbor inside (`cb` after `c`, `ab`
/// inside `b`), so every single letter stays free for the slot it names.
///
/// A letter never ends in `a`, because nothing sorts between `b` and `ba`: so there is always
/// room to insert another inside it.
pub fn planet_letter(
    placed: &[(String, f64)],
    semi_major_au: f64,
    luminosity_solar: f64,
    table: &[SpacingRow],
) -> String {
    let inside = placed
        .iter()
        .filter(|(_, a)| *a < semi_major_au)
        .max_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(letter, _)| letter.as_str());
    let outside = placed
        .iter()
        .filter(|(_, a)| *a >= semi_major_au)
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(letter, _)| letter.as_str());
    let slot = expected_slot(semi_major_au, luminosity_solar, table);
    let preferred = char::from(b'b' + slot.min((b'z' - b'b') as usize) as u8).to_string();
    let lo = inside.unwrap_or("a");
    // Letters received from other craft may not agree with this craft's orbits. Where the
    // neighbors are out of order the outer bound is dropped rather than looping on an
    // impossible gap.
    let hi = outside.filter(|hi| lo < *hi);
    let taken = placed.iter().any(|(letter, _)| *letter == preferred);
    if !taken && lo < preferred.as_str() && hi.is_none_or(|hi| preferred.as_str() < hi) {
        return preferred;
    }
    // A second letter after the neighbor inside, within its slot: bounded above by the next
    // single letter, so no single letter is ever spent on a planet out of its slot.
    let next = successor(lo.get(..1).unwrap_or("a"));
    let bound = match (hi, next.as_deref()) {
        (Some(hi), Some(next)) => Some(if hi < next { hi } else { next }),
        (hi, next) => hi.or(next),
    };
    between(lo, bound)
}

/// The string just after `s` at its own length: its last letter moved on one. `None` past `z`.
fn successor(s: &str) -> Option<String> {
    let mut bytes = s.as_bytes().to_vec();
    let last = bytes.last_mut()?;
    if *last >= b'z' {
        return None;
    }
    *last += 1;
    String::from_utf8(bytes).ok()
}

/// The shortest string strictly between `lo` and `hi`, over `a`–`z`, that does not end in `a`,
/// and the smallest of those, so planets found outward in order are lettered in order.
fn between(lo: &str, hi: Option<&str>) -> String {
    let lo = lo.as_bytes();
    let mut out = Vec::new();
    let mut lo_tight = true;
    let mut hi_tight = hi.is_some();
    for i in 0.. {
        let lo_c = lo.get(i).copied().filter(|_| lo_tight);
        let lo_done = lo_tight && lo_c.is_none();
        let hi_c = hi.and_then(|h| h.as_bytes().get(i).copied()).filter(|_| hi_tight);
        if hi_tight && hi_c.is_none() {
            // Only reachable if `hi <= lo`, which the caller rules out; stop constraining.
            hi_tight = false;
        }
        let above = |c: u8| !lo_tight || lo_done || lo_c.is_some_and(|l| c > l);
        let below = |c: u8| !hi_tight || hi_c.is_some_and(|h| c < h);
        let stops: Vec<u8> = (b'b'..=b'z').filter(|&c| above(c) && below(c)).collect();
        if let Some(&pick) = stops.first() {
            out.push(pick);
            break;
        }
        let c = lo_c.unwrap_or(b'a');
        out.push(c);
        if lo_done || lo_c.is_some_and(|l| c > l) {
            lo_tight = false;
        }
        if hi_c.is_some_and(|h| c < h) {
            hi_tight = false;
        }
    }
    // Every byte pushed is in `a`..=`z`.
    out.into_iter().map(char::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(pairs: &[(&str, f64)]) -> Vec<(String, f64)> {
        pairs.iter().map(|(l, a)| (l.to_string(), *a)).collect()
    }

    /// Around a sun-like star, the default table's slots: b 0.060, c 0.090, d 0.14, e 0.20 AU …
    #[test]
    fn a_planet_takes_the_letter_of_its_expected_slot() {
        assert_eq!(planet_letter(&[], 0.060, 1.0, SPACING), "b");
        assert_eq!(planet_letter(&[], 0.093, 1.0, SPACING), "c");
        assert_eq!(planet_letter(&[], 0.2235, 1.0, SPACING), "e");
        assert_eq!(planet_letter(&[], 0.005, 1.0, SPACING), "b", "nothing is inside b");
    }

    /// A Neptune found first far out leaves letters open inside it for what is expected there.
    #[test]
    fn gaps_are_left_for_planets_not_yet_found() {
        let first = planet_letter(&[], 30.0, 1.0, SPACING);
        assert!(first.as_str() > "f", "{first}");
        let later = planet_letter(&placed(&[(&first, 30.0)]), 0.093, 1.0, SPACING);
        assert_eq!(later, "c", "an inner planet found later takes its own slot");
    }

    #[test]
    fn a_hotter_star_expects_its_planets_further_out() {
        assert_eq!(planet_letter(&[], 0.60, 100.0, SPACING), "b");
        assert!(planet_letter(&[], 0.60, 1.0, SPACING).as_str() > "e");
    }

    /// A planet out of its slot takes a second letter, leaving the single letter for its slot.
    #[test]
    fn a_taken_slot_takes_a_second_letter_and_leaves_the_next_slot_free() {
        // Something at c's orbit is already "c"; a planet just outside it is still in c's slot.
        let one = placed(&[("c", 0.093)]);
        assert_eq!(planet_letter(&one, 0.105, 1.0, SPACING), "cb");
        let two = placed(&[("c", 0.093), ("cb", 0.105)]);
        assert_eq!(planet_letter(&two, 0.144, 1.0, SPACING), "d", "and d is still d's");
        // Something mis-slotted: "e" holds 0.08 AU, and a planet at 0.14 AU (slot d) is outside it.
        assert_eq!(planet_letter(&placed(&[("e", 0.08)]), 0.144, 1.0, SPACING), "eb");
    }

    /// No single letter left between two neighbors: a second one, never starting at `a`.
    #[test]
    fn too_few_gaps_add_a_second_letter() {
        let pair = placed(&[("b", 0.060), ("c", 0.093)]);
        assert_eq!(planet_letter(&pair, 0.070, 1.0, SPACING), "bb");
        let three = placed(&[("b", 0.060), ("bb", 0.070), ("c", 0.093)]);
        assert_eq!(planet_letter(&three, 0.080, 1.0, SPACING), "bc", "outward in order");
        assert_eq!(planet_letter(&three, 0.065, 1.0, SPACING), "bab", "and room inside bb too");
        assert_eq!(planet_letter(&placed(&[("b", 0.060)]), 0.02, 1.0, SPACING), "ab", "inside b");
    }

    /// Whatever the order of discovery, alphabetical order stays orbital order.
    #[test]
    fn alphabetical_order_is_orbital_order_whatever_was_found_first() {
        let orbits = [5.0, 0.4, 12.0, 1.1, 0.9, 2.2, 1.0, 30.0, 0.95, 0.5];
        let mut held: Vec<(String, f64)> = Vec::new();
        for a in orbits {
            let letter = planet_letter(&held, a, 1.0, SPACING);
            assert!(held.iter().all(|(l, _)| *l != letter), "{letter} given twice");
            held.push((letter, a));
        }
        let mut by_orbit = held.clone();
        by_orbit.sort_by(|x, y| x.1.total_cmp(&y.1));
        let mut by_letter = held.clone();
        by_letter.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(by_orbit, by_letter, "{held:?}");
    }

    /// If the table drifts from the generator, generated systems lettered in random order need
    /// second letters often, or leave the alphabet mostly empty.
    #[test]
    fn the_default_table_fits_what_the_generator_makes() {
        use crate::sky::{AuthoredStars, StarId, StarProvider, generate};

        let template = AuthoredStars::sample().stars()[1].clone();
        let (mut planets, mut doubled, mut widest) = (0usize, 0usize, 0u8);
        for key in 0..2000u64 {
            let mut star = template.clone();
            star.id = StarId::synthesise("letters", key);
            // The generator sizes a disc from the star's radius and temperature, not from a
            // luminosity column, so a star whose columns disagree is not a star it would ever
            // be handed. Build each one the way a catalogue does.
            let (lum, teff) = [(0.01, 3400.0), (0.3, 4900.0), (1.0, 5772.0), (5.0, 7600.0), (40.0, 11000.0)]
                [key as usize % 5];
            star.luminosity_solar = lum;
            star.star.teff_k = teff;
            star.star.radius_m = em_spectra::stellar::radius_from_luminosity(
                lum * em_spectra::stellar::SOLAR_LUMINOSITY,
                teff,
            );
            star.mass_solar = em_spectra::stellar::main_sequence_mass_solar(lum);
            star.star.mu = em_spectra::stellar::mu_from_mass_solar(star.mass_solar);
            let system = generate::system_for(&star);
            let mut orbits: Vec<f64> =
                system.planets.iter().map(|p| p.semi_major_m / 1.495_978_707e11).collect();
            // Found in whatever order, not outward.
            let n = orbits.len();
            for i in 0..n {
                orbits.swap(i, (crate::rng::hash(&[key, i as u64]) as usize) % n);
            }
            let mut held: Vec<(String, f64)> = Vec::new();
            for a in orbits {
                let letter = planet_letter(&held, a, star.luminosity_solar, SPACING);
                doubled += usize::from(letter.len() > 1);
                widest = widest.max(letter.as_bytes()[0]);
                held.push((letter, a));
                planets += 1;
            }
        }
        let rate = doubled as f64 / planets as f64;
        assert!(planets > 5000, "{planets}");
        assert!(rate < 0.15, "{:.1}% of planets needed a second letter", rate * 100.0);
        assert!(widest < b'u', "letters reached {}", widest as char);
    }

    #[test]
    fn a_small_body_is_named_for_when_it_was_found() {
        assert_eq!(discovery_designation(2.5 * JULIAN_YEAR_S, 3), "2-3");
        assert_eq!(discovery_designation(0.0, 1), "0-1");
    }

    #[test]
    fn a_designation_says_which_way_it_was_found() {
        let along_x = designation(DVec3::X);
        let up = designation(DVec3::Z);
        assert_ne!(along_x, up);
        assert!(up.contains("+90"), "{up}");
        assert_eq!(designation(DVec3::X * 3.0), along_x, "a direction, not a distance");
    }
}

