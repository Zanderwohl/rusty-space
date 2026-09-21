//! What drawing the sky costs, at rest and at speed.
//!
//! A profile of a frozen browser put 90% of the frame inside two siblings under one parent,
//! with no recursion and a flat stack — the shape of a loop over something large rather than a
//! pathological function. The sky is the only per-frame loop over thousands of things, and
//! `radiance_from` shifts each star's temperature by its own Doppler factor, so nothing about
//! it can be cached while the ship is moving.

use std::time::Instant;

use glam::DVec3;
use lc_client::session::Session;
use lc_world::sky::{AuthoredStars, CatalogueStar, StarId, StarProvider};

/// About what the shipped catalogue chunk holds.
const STARS: u64 = 8000;

fn a_full_sky() -> AuthoredStars {
    let one = AuthoredStars::sample().stars()[1].clone();
    let stars: Vec<CatalogueStar> = (0..STARS)
        .map(|n| {
            let mut star = one.clone();
            star.id = StarId::synthesise("bench", n);
            // Spread them over a few hundred light-years, off-axis so nothing degenerates.
            let f = n as f64;
            star.position_ly = DVec3::new(f % 97.0 - 48.0, f % 61.0 - 30.0, f % 43.0 - 21.0);
            star
        })
        .collect();
    AuthoredStars::new("bench", stars)
}

#[test]
fn the_sky_costs_the_same_moving_as_it_does_at_rest() {
    let mut session = Session::new(&a_full_sky(), STARS as usize);
    assert!(
        session.stars.len() > 1000,
        "premise: a full sky, not three stars"
    );

    let time = |session: &Session| {
        let started = Instant::now();
        let drawn = session.sky();
        let each = started.elapsed();
        assert_eq!(drawn.len(), session.stars.len());
        each
    };

    let at_rest = time(&session);
    // Two tenths of `c`, which is where a cut part-way through a crossing leaves a ship. Every
    // star's temperature is Doppler-shifted by its own factor, so this is the case a cache
    // keyed on temperature could plausibly miss on every frame — and it must not.
    session.ship.motion.beta = DVec3::new(0.2, 0.0, 0.0);
    let moving = time(&session);

    println!("sky at rest: {at_rest:?}   at 0.2c: {moving:?}");

    // Asserted only in an optimized build, because that is the one the claim is about: a
    // browser runs optimized wasm, and an unoptimized build is ten times slower, so a bound it
    // could meet would be no bound at all. Before the spectra were reused this was 7.2 ms.
    #[cfg(not(debug_assertions))]
    assert!(
        moving < std::time::Duration::from_millis(3),
        "drawing the sky while moving costs {moving:?}, and used to cost 7.2 ms",
    );
}
