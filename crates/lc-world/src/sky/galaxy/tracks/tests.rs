use super::*;
use super::giants;

/// Tout et al.'s fit, evaluated by hand from its table at Z = 0.02.
#[test]
fn the_zams_sun_matches_the_tout_table() {
    let c = Coefficients::new(0.02);
    assert!((c.l_zams(1.0) - 0.6977).abs() < 5e-4, "{}", c.l_zams(1.0));
    assert!((c.r_zams(1.0) - 0.8883).abs() < 5e-4, "{}", c.r_zams(1.0));
}

/// The Sun at 4.57 Gyr comes out near one solar luminosity and radius; Hurley et al. calibrated
/// against solar models, so a broken coefficient shows here first.
#[test]
fn the_sun_today_is_near_one_solar_unit() {
    let e = evolve(1.0, 4.57, 0.0);
    assert_eq!(e.state, State::MainSequence);
    assert!((e.luminosity_solar - 1.0).abs() < 0.1, "L = {}", e.luminosity_solar);
    assert!((e.radius_solar - 1.0).abs() < 0.1, "R = {}", e.radius_solar);
    assert!((e.teff_k - 5772.0).abs() < 200.0, "T = {}", e.teff_k);
}

#[test]
fn lifetimes_fall_with_mass_and_the_sun_lives_about_ten_gyr() {
    let c = Coefficients::new(0.02);
    let t_sun = c.t_ms(1.0) / 1000.0;
    assert!((9.0..12.5).contains(&t_sun), "t_MS(1 M☉) = {t_sun} Gyr");
    let masses = [0.5, 0.8, 1.0, 2.0, 5.0, 10.0, 30.0, 100.0];
    for w in masses.windows(2) {
        assert!(c.t_ms(w[0]) > c.t_ms(w[1]), "{} vs {}", w[0], w[1]);
    }
    assert!(c.t_ms(30.0) < 10.0, "a 30 M☉ star lives a few Myr");
}

/// Luminosity grows across the main sequence, and metal-poor stars are hotter and brighter at
/// a given mass.
#[test]
fn main_sequence_stars_brighten_and_metal_poor_ones_run_hotter() {
    for m in [0.3, 0.8, 1.0, 1.5, 3.0, 10.0] {
        let c = Coefficients::new(0.02);
        let t = c.t_ms(m);
        let (l0, _) = c.main_sequence(m, 0.0);
        let (l1, _) = c.main_sequence(m, 0.95 * t);
        assert!(l1 > l0, "M = {m}");
        assert!((l0 / c.l_zams(m) - 1.0).abs() < 1e-9, "starts on the ZAMS at M = {m}");
    }
    let rich = evolve(1.0, 0.0, 0.0);
    let poor = evolve(1.0, 0.0, -1.5);
    assert!(poor.teff_k > rich.teff_k + 300.0 && poor.luminosity_solar > rich.luminosity_solar);
}

#[test]
fn every_mass_and_metallicity_gives_finite_stars_at_every_age() {
    for feh in [-2.5, -1.0, 0.0, 0.2] {
        for i in 0..60 {
            let m = 0.08 * (150.0f64 / 0.08).powf(i as f64 / 59.0);
            for age in [0.0, 0.01, 0.1, 1.0, 5.0, 13.0] {
                let e = evolve(m, age, feh);
                for v in [e.luminosity_solar, e.teff_k, e.radius_solar, e.mass_solar] {
                    assert!(v.is_finite() && v >= 0.0, "M = {m}, age = {age}, [Fe/H] = {feh}: {e:?}");
                }
                if e.state == State::MainSequence {
                    assert!(e.teff_k > 1500.0 && e.teff_k < 70_000.0, "{m} {age} {feh}: {e:?}");
                }
            }
        }
    }
}

/// A Sun's whole life: a subgiant past 11 Gyr, a giant branch to a tip near 2000–3000 L☉, a
/// horizontal branch near 50 L☉ for about 100 Myr, an AGB, and a white dwarf near 0.58 M☉.
#[test]
fn a_sun_lives_out_its_life() {
    let c = Coefficients::new(0.02);
    let life = giants::Life::new(&c, 1.0);
    assert!(life.t_ms < life.t_bgb && life.t_bgb < life.t_end, "{} {} {}", life.t_ms, life.t_bgb, life.t_end);
    let states: Vec<State> = (0..400).map(|i| evolve(1.0, 10.9 + i as f64 * 0.005, 0.0).state).collect();
    for want in [State::MainSequence, State::HertzsprungGap, State::GiantBranch, State::CoreHeliumBurning, State::AsymptoticGiantBranch, State::WhiteDwarf] {
        assert!(states.contains(&want), "never {want:?}");
    }
    let peak = (0..4000)
        .map(|i| evolve(1.0, 11.0 + i as f64 * 0.0005, 0.0))
        .filter(|e| e.state == State::GiantBranch)
        .map(|e| e.luminosity_solar)
        .fold(0.0, f64::max);
    assert!((1500.0..4000.0).contains(&peak), "tip of the giant branch {peak} L☉");
    let hb: Vec<f64> = (0..4000)
        .map(|i| evolve(1.0, 11.0 + i as f64 * 0.0005, 0.0))
        .filter(|e| e.state == State::CoreHeliumBurning)
        .map(|e| e.luminosity_solar)
        .collect();
    assert!(!hb.is_empty() && hb.len() as f64 * 0.5 > 50.0 && (hb.len() as f64 * 0.5) < 200.0, "{} Myr of helium burning", hb.len() as f64 * 0.5);
    assert!(hb.iter().all(|l| (20.0..200.0).contains(l)), "{hb:?}");
    let wd = evolve(1.0, 13.5, 0.0);
    assert_eq!(wd.state, State::WhiteDwarf);
    assert!((wd.mass_solar - 0.58).abs() < 0.03 && (wd.radius_solar - 0.0126).abs() < 0.003, "{wd:?}");
}

/// Hurley et al. (2000) Table 1: the giant branch lasts a fraction of t_BGB that falls with mass.
#[test]
fn giant_branch_durations_match_table_one() {
    let c = Coefficients::new(0.02);
    for (m, want) in [(1.0, 6.4e-2), (2.0, 2.0e-2), (5.0, 2.4e-3)] {
        let life = giants::Life::new(&c, m);
        let got = (life.t_hei() - life.t_bgb) / life.t_bgb;
        assert!((got / want - 1.0).abs() < 0.25, "M = {m}: {got} against {want}");
    }
}

/// What a star leaves: an ONe white dwarf at 7 M☉, neutron stars at 10 and 20 M☉, a black hole
/// at 40 M☉.
#[test]
fn massive_stars_end_as_the_core_mass_says() {
    let ends: Vec<(f64, End)> = [7.0, 10.0, 20.0, 40.0]
        .iter()
        .map(|&m| (m, giants::Life::new(&Coefficients::new(0.02), m).end))
        .collect();
    assert!(matches!(ends[0].1, End::WhiteDwarf { mass } if (0.9..1.44).contains(&mass)), "{:?}", ends[0]);
    assert!(matches!(ends[1].1, End::NeutronStar { mass } if (1.25..1.5).contains(&mass)), "{:?}", ends[1]);
    assert!(matches!(ends[2].1, End::NeutronStar { mass } if (1.5..1.85).contains(&mass)), "{:?}", ends[2]);
    assert!(matches!(ends[3].1, End::BlackHole { .. }), "{:?}", ends[3]);
    assert_eq!(evolve(20.0, 0.1, 0.0).state, State::NeutronStar);
}

/// A 5 M☉ star's radius dips during helium burning: the blue loop.
#[test]
fn intermediate_stars_make_a_blue_loop() {
    let c = Coefficients::new(0.02);
    let life = giants::Life::new(&c, 5.0);
    let r: Vec<(State, f64)> = (0..2000)
        .map(|i| life.t_ms + (life.t_end - life.t_ms) * i as f64 / 2000.0)
        .map(|t| life.at(t))
        .map(|e| (e.state, e.radius_solar))
        .collect();
    let giant_max = r.iter().filter(|(s, _)| *s == State::GiantBranch || *s == State::HertzsprungGap).map(|x| x.1).fold(0.0, f64::max);
    let loop_min = r.iter().filter(|(s, _)| *s == State::CoreHeliumBurning).map(|x| x.1).fold(f64::INFINITY, f64::min);
    assert!(loop_min < 0.6 * giant_max, "loop {loop_min} R☉ against giant {giant_max} R☉");
}

/// The cached coefficients for a record's `[Fe/H]` step give the same star as building them.
#[test]
fn a_cached_step_evolves_as_built() {
    for step in [-128i8, -50, 0, 7, 127] {
        let feh = step as f64 * crate::sky::galaxy::FEH_STEP;
        for (m, age) in [(0.09, 1.0), (1.0, 4.57), (1.0, 12.3), (5.0, 0.11), (20.0, 0.1)] {
            assert_eq!(evolve_step(m, age, step), evolve(m, age, feh), "step {step}, M = {m}, age = {age}");
        }
    }
}

/// Below 0.1 M☉ the main sequence extrapolates with eq. 24's floor instead of copying 0.1 M☉.
#[test]
fn the_lightest_stars_are_not_all_one_star() {
    let a = evolve(0.08, 1.0, 0.0);
    let b = evolve(0.1, 1.0, 0.0);
    assert!(a.luminosity_solar < b.luminosity_solar, "{a:?} {b:?}");
    assert!(a.radius_solar > 0.0 && a.teff_k > 1000.0, "{a:?}");
}
