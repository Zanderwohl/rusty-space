//! How far something is, as far as this ship knows, in words.
//!
//! A distance comes from bearings taken a baseline apart or from a claim; until then a star is
//! only a direction. Nothing here reads the catalog. See `lightcone/docs/22-provenance.md`.

use lc_world::knowledge::observatory::CHARTS;
use lc_world::knowledge::{Belief, Distance, Witness};

const ARCSEC_PER_RAD: f64 = 206_264.806;

pub fn who(witness: Witness, owner: Witness) -> String {
    match witness {
        w if w == owner => "this ship".into(),
        CHARTS => "the charts".into(),
        Witness(n) => format!("craft {n}"),
    }
}

/// `None` is a star this ship has never detected. Where the range came from is [`sources`].
pub fn short(belief: Option<&Belief>, here_ly: glam::DVec3) -> String {
    let Some(belief) = belief else { return "not detected".into() };
    match belief.distance {
        Distance::Unknown => "bearing only".into(),
        Distance::AtLeast(ly) => format!("beyond {ly:.1} ly"),
        Distance::Measured { position_ly, sigma_ly } => {
            format!("{:.2} ± {sigma_ly:.2} ly", position_ly.distance(here_ly))
        }
    }
}

pub fn sources(belief: &Belief, owner: Witness) -> Vec<String> {
    let instruments = if belief.witnesses == 1 { "instrument" } else { "instruments" };
    let mut notes = vec![format!("{} bearings from {} {instruments}", belief.sightings, belief.witnesses)];
    if let Some(angle) = belief.baseline_rad {
        notes.push(format!("Bearings {} apart", angle_text(angle)));
    }
    notes.push(match belief.hops {
        0 => "Seen from this ship".into(),
        1 => "Relayed once; somebody else did the looking".into(),
        n => format!("Relayed {n} times"),
    });
    // A distance solved from bearings can be checked here; a stated one cannot, however narrow
    // its error.
    notes.push(match (belief.distance, belief.claimed_by) {
        (Distance::Unknown, _) => "No parallax yet".into(),
        (Distance::AtLeast(ly), _) => format!("No parallax over the baseline, so past {ly:.1} ly"),
        (Distance::Measured { .. }, Some(whose)) => format!("Distance on {} word", possessive(&who(whose, owner))),
        (Distance::Measured { .. }, None) if belief.triangulated => "Distance solved from bearings held here".into(),
        (Distance::Measured { .. }, None) => "Distance on somebody else's word".into(),
    });
    if let Some(floor) = belief.floor_ly {
        notes.push(format!("This ship's own bearings put it beyond {floor:.1} ly"));
    }
    notes
}

fn possessive(name: &str) -> String {
    if name.ends_with('s') { format!("{name}'") } else { format!("{name}'s") }
}

fn angle_text(rad: f64) -> String {
    let arcsec = rad * ARCSEC_PER_RAD;
    match arcsec {
        a if a < 1.0 => format!("{:.0} mas", a * 1.0e3),
        a if a < 60.0 => format!("{a:.1}″"),
        a if a < 3600.0 => format!("{:.1}′", a / 60.0),
        a => format!("{:.1}°", a / 3600.0),
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_world::knowledge::{Bearing, Claim, Knowledge, Sighting};
    use lc_world::sky::StarId;

    use super::*;

    fn look(from: DVec3, star: DVec3, t: f64) -> Sighting {
        Sighting {
            witness: Witness(1),
            observed_s: t,
            bearing: Bearing { observer_ly: from, toward: (star - from).normalize(), sigma_rad: 1e-9 },
            band: em_spectra::Band::V,
            flux: 1e-12,
            flux_sigma: 1e-15,
            lineage: Vec::new(),
        }
    }

    /// A range this ship measured names its baseline; a bare direction has no range.
    #[test]
    fn a_range_says_where_it_came_from() {
        let id = StarId::synthesise("range", 1);
        let star = DVec3::new(0.0, 0.0, 4.0);
        let mut k = Knowledge::new(Witness(1));
        assert_eq!(short(k.belief(id), DVec3::ZERO), "not detected");

        k.sighted(id, look(DVec3::ZERO, star, 0.0));
        assert_eq!(short(k.belief(id), DVec3::ZERO), "bearing only");

        k.sighted(id, look(DVec3::X * 1.0e-3, star, 1.0));
        let belief = k.belief(id).unwrap();
        assert!(short(Some(belief), DVec3::ZERO).starts_with("4.00"));
        let notes = sources(belief, Witness(1));
        assert!(notes.iter().any(|n| n.starts_with("Bearings ") && n.ends_with(" apart")), "{notes:?}");
        assert!(notes.iter().any(|n| n == "Distance solved from bearings held here"), "{notes:?}");
    }

    /// The short form is only the number; whose word it is goes to the sources.
    #[test]
    fn a_short_range_leaves_its_sources_apart() {
        let id = StarId::synthesise("range", 2);
        let star = DVec3::new(0.0, 0.0, 4.0);
        let mut k = Knowledge::new(Witness(1));
        k.sighted(id, look(DVec3::ZERO, star, 0.0));
        k.told(id, Claim { witness: CHARTS, distance: Distance::Measured { position_ly: star, sigma_ly: 0.04 }, stated_s: 0.0, lineage: Vec::new() });
        let belief = k.belief(id).unwrap();
        assert_eq!(short(Some(belief), DVec3::ZERO), "4.00 ± 0.04 ly");
        let notes = sources(belief, Witness(1));
        assert!(notes.iter().any(|n| n == "Distance on the charts' word"), "{notes:?}");
        assert!(notes.iter().any(|n| n == "Seen from this ship"), "{notes:?}");
    }
}
