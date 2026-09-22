//! How far something is, as far as this ship knows, in words.
//!
//! A distance is earned: it comes from bearings taken a baseline apart, or on somebody's word,
//! and until then a star is a direction. Nothing here reads the catalogue. See
//! `lightcone/docs/22-provenance.md`.

use lc_world::knowledge::observatory::CHARTS;
use lc_world::knowledge::{Belief, Distance, Witness};

/// Arc-seconds in a radian, for a baseline small enough to want them.
const ARCSEC_PER_RAD: f64 = 206_264.806;

/// Who a witness is, from this ship.
pub fn who(witness: Witness, owner: Witness) -> String {
    match witness {
        w if w == owner => "this ship".into(),
        CHARTS => "the charts".into(),
        Witness(n) => format!("craft {n}"),
    }
}

/// The range to a believed star from `here_ly`, with its error and where it came from, or what
/// is known instead. `None` for something this ship has never detected.
pub fn describe(belief: Option<&Belief>, owner: Witness, here_ly: glam::DVec3) -> String {
    let mut text = short(belief, here_ly);
    let Some(belief) = belief else { return text };
    if matches!(belief.distance, Distance::Measured { .. }) {
        let whence = match (belief.claimed_by, belief.baseline_rad) {
            (Some(whose), _) => format!("on {} word", possessive(&who(whose, owner))),
            (None, Some(angle)) => format!("from bearings a {} baseline apart", angle_text(angle)),
            (None, None) => "from bearings".into(),
        };
        text += &format!(", {whence}");
        if let Some(floor) = belief.floor_ly {
            text += &format!(" — though this ship's own bearings put it beyond {floor:.1} ly");
        }
    }
    text
}

/// The range alone, with its error: where it came from is [`sources`].
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

/// Where a belief came from, one note per line.
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
    // A distance worked out from bearings is one this ship can check. A stated one is not,
    // however narrow its error.
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

/// An angle, in whatever unit reads best.
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

    /// A direction is not a range, a claim says whose it is, and a range this ship measured
    /// says what baseline it had to work with.
    #[test]
    fn a_range_says_where_it_came_from() {
        let id = StarId::synthesise("range", 1);
        let star = DVec3::new(0.0, 0.0, 4.0);
        let mut k = Knowledge::new(Witness(1));
        assert_eq!(describe(k.belief(id), Witness(1), DVec3::ZERO), "not detected");

        k.sighted(id, look(DVec3::ZERO, star, 0.0));
        assert_eq!(describe(k.belief(id), Witness(1), DVec3::ZERO), "bearing only");

        k.told(id, Claim { witness: CHARTS, distance: Distance::Measured { position_ly: star, sigma_ly: 0.04 }, stated_s: 0.0, lineage: Vec::new() });
        assert!(describe(k.belief(id), Witness(1), DVec3::ZERO).contains("on the charts' word"));

        k.sighted(id, look(DVec3::X * 1.0e-3, star, 1.0));
        let text = describe(k.belief(id), Witness(1), DVec3::ZERO);
        assert!(text.starts_with("4.00") && text.contains("baseline apart"), "{text}");
    }

    /// The short form is a number and nothing else; where it came from goes to the sources.
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
