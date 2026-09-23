//! What a body in the local system looks like from a ship inside it: which way, how big, and
//! how bright in every band the sensor has.
//!
//! The counterpart of [`crate::observation`], which watches a whole system from light-years off
//! and sees only the star's light going missing. Here the bodies are resolved and measured one
//! at a time. See `lightcone/docs/25-system-knowledge.md`, "From inside: surveying a system".
//!
//! Truth only. Nothing here reads or writes a belief; it answers what arrives at a point, and
//! the survey decides what a telescope makes of it.

use em_spectra::{Band, PerBand, blackbody};
use glam::DVec3;

use crate::knowledge::survey;
use crate::star::Star;
use crate::system::{Drawable, LocalSystem, M_PER_LY, phase_factor};

/// One body as its light arrives at an observer.
#[derive(Clone, Debug, PartialEq)]
pub struct Visit {
    /// The body's own key, which is what [`crate::navigation::Target::Body`] holds and so what
    /// `BodyId::of(star, key)` hashes. Not the `em-sim` arena id that `worlds` and `rings` are
    /// keyed by; the two differ and [`Drawable`] carries this one.
    pub body: String,
    /// Unit vector from the observer.
    pub toward: DVec3,
    pub range_m: f64,
    /// Angular diameter, radians. Above the optics' resolution the disc is resolved and its
    /// size is a measurement; below it the body is a point like any star.
    pub diameter_rad: f64,
    /// Star-body-observer angle. Zero is full phase; `PI` is a body between ship and star,
    /// lit entirely from behind.
    pub phase_rad: f64,
    /// W/m^2 arriving in each band: reflected starlight plus the body's own thermal emission.
    pub flux: PerBand<f64>,
}

/// What arrives from `body` at `from_ly`, with the star at `star_at_ly`.
///
/// Reflected light uses the **band** flux falling on the body and its geometric albedo in that
/// band, which is what a geometric albedo is defined against: at full phase a body returns
/// `incident * albedo * (radius / range)^2`, with no factor of pi. Thermal emission is the
/// geometry [`survey::flux_from`] uses for a star, at the body's own radiating temperature and
/// not one derived from its distance, because a giant makes its own heat.
///
/// Rings reflect too, over whatever of their cross-section is turned toward both the star and
/// the observer, at their own albedo. Saturn's ice is brighter than Saturn.
pub fn of(star: &Star, star_at_ly: DVec3, body: &Drawable, from_ly: DVec3) -> Visit {
    let offset = body.position_ly - from_ly;
    let range_m = offset.length() * M_PER_LY;
    let orbit_m = (body.position_ly - star_at_ly).length() * M_PER_LY;
    let to_star = star_at_ly - body.position_ly;
    let to_ship = from_ly - body.position_ly;
    let lit = phase_factor(to_star, to_ship);
    let disc = std::f64::consts::PI * body.radius_m * body.radius_m * lit;
    let ring_m2 = body.rings.as_ref().map_or(0.0, |rings| {
        let toward_star = rings.pole.dot(to_star.normalize_or_zero()).abs();
        let toward_ship = rings.pole.dot(to_ship.normalize_or_zero()).abs();
        rings.system.cross_section_m2() * toward_star * toward_ship
    });
    let ring_albedo = body.rings.as_ref().map_or(0.0, |rings| rings.system.albedo);

    let flux = PerBand::new(std::array::from_fn(|i| {
        let band = Band::ALL[i];
        if range_m <= 0.0 {
            return 0.0;
        }
        // A geometric albedo is defined against a flat disc of the body's own radius, so the
        // reflecting area divides by that same disc to stay in those units.
        let full = std::f64::consts::PI * body.radius_m * body.radius_m;
        let reflected = if orbit_m > 0.0 && full > 0.0 {
            let returned = body.world.reflectance_in(band) * disc + ring_albedo * ring_m2;
            survey::flux_from(star, band, orbit_m) * returned / full
                * (body.radius_m / range_m)
                * (body.radius_m / range_m)
        } else {
            0.0
        };
        // Radiated from the whole sphere and not from the lit crescent, so no phase factor:
        // this is the one signal a body still gives on its night side.
        let thermal = std::f64::consts::PI
            * blackbody::band_radiance(band, body.effective_k)
            * body.world.emissivity(band)
            * (body.radius_m / range_m)
            * (body.radius_m / range_m);
        reflected + thermal
    }));

    Visit {
        body: body.name.clone(),
        toward: offset.normalize_or_zero(),
        range_m,
        diameter_rad: if range_m > 0.0 { 2.0 * body.radius_m / range_m } else { 0.0 },
        phase_rad: to_star
            .normalize_or_zero()
            .angle_between(to_ship.normalize_or_zero()),
        flux,
    }
}

/// Every body of a system as its light arrives at `from_ly`, brightest in `band` first.
///
/// Ordered because the glare rules in [`survey`] are asked which of two sources is brighter,
/// and a caller that has already sorted need not re-derive it.
pub fn all(system: &LocalSystem, band: Band, from_ly: DVec3, now_s: f64) -> Vec<Visit> {
    let star = Star {
        radius_m: system.star_radius_m(),
        teff_k: system.star_teff_k(),
        mu: 0.0,
        limb_darkening: (0.0, 0.0),
    };
    let at = system.star_position_at(now_s).unwrap_or(system.star_position_ly());
    let mut seen: Vec<Visit> = system
        .drawables_at(from_ly, now_s)
        .iter()
        .map(|body| of(&star, at, body, from_ly))
        .collect();
    seen.sort_unstable_by(|a, b| b.flux[band].total_cmp(&a.flux[band]));
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{StarProvider, hyg::HygProvider};

    const AU_M: f64 = 1.495_978_707e11;

    fn catalogue() -> Option<HygProvider> {
        HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv").ok()
    }

    fn sol() -> Option<LocalSystem> {
        let provider = catalogue()?;
        let sun = provider
            .stars()
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some(crate::system::SOL))?
            .clone();
        LocalSystem::for_star(&sun)
    }

    fn star_of(system: &LocalSystem) -> Star {
        Star {
            radius_m: system.star_radius_m(),
            teff_k: system.star_teff_k(),
            mu: 0.0,
            limb_darkening: (0.0, 0.0),
        }
    }

    /// `range_au` from `name` on the star's side of it, so the body is at full phase and at a
    /// known range. Where a planet happens to be in its orbit is then not part of the answer.
    ///
    /// The star's side, not the far side: the ship has to be between the two to see a lit face.
    /// The far side is conjunction, and that is [`a_body_lit_from_behind_keeps_only_its_own_heat`].
    fn full_phase(system: &LocalSystem, name: &str, range_au: f64) -> Option<(Drawable, Visit)> {
        let star_at = system.star_position_ly();
        let body = system
            .drawables_at(star_at, 0.0)
            .into_iter()
            .find(|b| b.name == name)?;
        let out = (body.position_ly - star_at).normalize();
        let from = body.position_ly - out * range_au * AU_M / M_PER_LY;
        let seen = of(&star_of(system), star_at, &body, from);
        Some((body, seen))
    }

    /// **The number that says the reflected-light relation is right.** A geometric albedo is
    /// defined against a flat disc of the body's own radius, so the flux is
    /// `incident * albedo * (radius / range)^2` -- easy to get wrong by a factor of four or of
    /// pi in either direction, and the check is independent: Jupiter at opposition is V = -2.7,
    /// which is 2.7e-8 W/m^2 through a V filter.
    #[test]
    fn jupiter_comes_out_at_the_brightness_it_is_seen_at() {
        let Some(system) = sol() else { return };
        let Some((body, seen)) = full_phase(&system, "Jupiter", 5.0) else {
            panic!("the preset carries Jupiter")
        };
        assert!(seen.phase_rad < 1.0e-6, "placed at full phase, got {}", seen.phase_rad);
        assert!(
            (seen.range_m / (5.0 * AU_M) - 1.0).abs() < 1.0e-9,
            "placed 5 AU off, got {} AU",
            seen.range_m / AU_M
        );
        let v = seen.flux[Band::V];
        assert!(
            (v / 2.6e-8 - 1.0).abs() < 0.25,
            "{v:e} W/m^2 in V against 2.6e-8 read off its magnitude"
        );
        // Its thermal bands are its own, not reflected, so they survive the terminator.
        assert!(seen.flux[Band::ThermalIr] > 0.0 && seen.flux[Band::Radio] > 0.0);
        assert!(body.radius_m > 6.0e7, "the preset's Jupiter is a giant");
    }

    /// The three statements a survey reads off the optical run, now as arriving flux rather
    /// than as table entries: Venus the bright one, Mars the dark one, Earth blue.
    #[test]
    fn the_inner_planets_arrive_ordered_and_colored_as_they_are_seen() {
        let Some(system) = sol() else { return };
        // The same range and the same phase for all three, so only the bodies differ.
        let at = |name: &str| full_phase(&system, name, 2.0).map(|(_, seen)| seen);
        let (Some(venus), Some(earth), Some(mars)) = (at("Venus"), at("Earth"), at("Mars")) else {
            panic!("the preset carries the inner planets")
        };

        // Scaled to a common radius, because Mars is half the size of the other two and this
        // is a claim about their surfaces.
        let brightness = |seen: &Visit, radius_m: f64| {
            seen.flux[Band::V] * (1.0 / (radius_m * radius_m))
        };
        let (rv, re, rm) = (6.05e6, 6.37e6, 3.39e6);
        assert!(
            brightness(&venus, rv) > brightness(&earth, re),
            "Venus is the bright one"
        );
        assert!(
            brightness(&earth, re) > brightness(&mars, rm),
            "Mars is the dark one"
        );

        let color = |seen: &Visit| seen.flux[Band::R] / seen.flux[Band::B];
        assert!(color(&earth) < color(&venus), "Earth is bluer than Venus");
        assert!(color(&mars) > color(&venus), "Mars is redder than Venus");
    }

    /// From five AU every major planet is a disc many resolution elements across, which is what
    /// makes a survey from inside looking rather than statistics at the noise floor.
    #[test]
    fn a_planet_seen_from_inside_is_resolved_outright() {
        let Some(system) = sol() else { return };
        let optics = crate::knowledge::survey::Optics::of(crate::instrument::Instrument::SHIP);
        let resolution = optics.resolution_rad(Band::V);

        for (name, arcsec) in [("Jupiter", 38.6), ("Saturn", 17.8), ("Earth", 3.5), ("Mars", 1.87)] {
            let range = if name == "Saturn" { 9.0 } else { 5.0 };
            let Some((_, seen)) = full_phase(&system, name, range) else { panic!("{name} is missing") };
            let seen_arcsec = seen.diameter_rad * 206_265.0;
            assert!(
                (seen_arcsec / arcsec - 1.0).abs() < 0.1,
                "{name} is {seen_arcsec:.2} arcsec, not {arcsec}"
            );
            assert!(
                seen.diameter_rad > resolution * 20.0,
                "{name} is {} resolution elements and should be resolved",
                seen.diameter_rad / resolution
            );
        }
    }

    /// Saturn's ice is brighter than Saturn: the rings reflect over whatever of their
    /// cross-section is turned toward both the star and the ship, and dropping them would make
    /// the one body in the system with rings the one whose brightness is wrong.
    #[test]
    fn rings_reflect_and_saturn_is_brighter_for_them() {
        let Some(system) = sol() else { return };
        let star_at = system.star_position_ly();
        let Some(mut body) = system
            .drawables_at(star_at, 0.0)
            .into_iter()
            .find(|b| b.name == "Saturn")
        else {
            panic!("the preset carries Saturn")
        };
        assert!(body.rings.is_some(), "Saturn has rings in the table");

        let out = (body.position_ly - star_at).normalize();
        let from = body.position_ly + out * 5.0 * AU_M / M_PER_LY;
        let with = of(&star_of(&system), star_at, &body, from);
        body.rings = None;
        let without = of(&star_of(&system), star_at, &body, from);
        assert!(
            with.flux[Band::V] > without.flux[Band::V],
            "{:e} with rings against {:e} without",
            with.flux[Band::V],
            without.flux[Band::V]
        );
    }

    /// A giant does not radiate at the temperature its distance implies, so its thermal flux
    /// cannot be predicted from where it is. Jupiter is the measurement.
    #[test]
    fn a_giant_radiates_more_than_its_distance_allows() {
        let Some(system) = sol() else { return };
        let Some((body, seen)) = full_phase(&system, "Jupiter", 5.0) else { panic!("no Jupiter") };
        assert!(
            body.effective_k > body.equilibrium_k,
            "{} K radiating against {} K from sunlight alone",
            body.effective_k,
            body.equilibrium_k
        );
        let cold = std::f64::consts::PI
            * blackbody::band_radiance(Band::ThermalIr, body.equilibrium_k)
            * body.world.emissivity(Band::ThermalIr)
            * (body.radius_m / seen.range_m)
            * (body.radius_m / seen.range_m);
        assert!(
            seen.flux[Band::ThermalIr] > cold,
            "its own heat has to show: {:e} against {cold:e}",
            seen.flux[Band::ThermalIr]
        );
    }

    /// **The key the whole survey joins on.** A `Visit` is turned into a `BodyId` through
    /// `BodyId::of(star, key)`, and the key has to be the one the inventory holds -- not the
    /// `em-sim` arena id that `worlds` and `rings` are keyed by, which is a different string
    /// for Luna among others.
    #[test]
    fn a_visit_is_keyed_the_way_the_inventory_is() {
        let Some(system) = sol() else { return };
        let seen = all(&system, Band::V, system.star_position_ly(), 0.0);
        assert!(seen.len() > 100, "only {} bodies", seen.len());

        let keys: std::collections::HashSet<&str> = system
            .inventory()
            .iter()
            .filter_map(|e| match &e.target {
                crate::navigation::Target::Body(key) => Some(key.as_str()),
                _ => None,
            })
            .collect();
        for visit in &seen {
            assert!(
                keys.contains(visit.body.as_str()),
                "{} is in no inventory entry, so nothing can name it",
                visit.body
            );
        }
    }

    /// Brightest first, because the glare rules are asked which of two sources outshines the
    /// other and re-deriving that per pair is how the order goes wrong.
    #[test]
    fn the_brightest_body_comes_first() {
        let Some(system) = sol() else { return };
        let from = system.star_position_ly() + DVec3::X * 5.0 * AU_M / M_PER_LY;
        let seen = all(&system, Band::V, from, 0.0);
        assert!(
            seen.windows(2).all(|w| w[0].flux[Band::V] >= w[1].flux[Band::V]),
            "the order is not by brightness"
        );
    }

    /// Nothing lit points this way at opposition, and yet the body is still there in the
    /// thermal bands. Which is why Mercury and Venus near conjunction stay hard.
    #[test]
    fn a_body_lit_from_behind_keeps_only_its_own_heat() {
        let Some(system) = sol() else { return };
        let star_at = system.star_position_ly();
        let Some(body) = system
            .drawables_at(star_at, 0.0)
            .into_iter()
            .find(|b| b.name == "Venus")
        else {
            panic!("the preset carries Venus")
        };
        // Between the ship and the star: the ship is outside Venus's orbit, looking inward.
        let inward = (body.position_ly - star_at).normalize();
        let from = star_at + inward * 5.0 * AU_M / M_PER_LY;
        let seen = of(&star_of(&system), star_at, &body, from);
        assert!(
            seen.phase_rad > std::f64::consts::PI - 1.0e-6,
            "placed at conjunction, got {} rad",
            seen.phase_rad
        );
        // Against its own full-phase flux rather than an absolute floor: `acos` near -1 keeps
        // only half its digits, so the phase lands a hair off PI and the factor is that hair
        // rather than zero.
        let (_, full) = full_phase(&system, "Venus", 5.0).expect("Venus again");
        assert!(
            seen.flux[Band::V] < full.flux[Band::V] * 1.0e-6,
            "no lit face: {:e} against {:e} full",
            seen.flux[Band::V],
            full.flux[Band::V]
        );
        assert!(
            seen.flux[Band::Radio] > 0.0,
            "radio comes from the surface and does not care about the phase"
        );
    }
}
