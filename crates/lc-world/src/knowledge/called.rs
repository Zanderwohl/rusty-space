//! What a craft calls a body nobody has named: its designation, read after the most the craft
//! can say about what the body is.
//!
//! Derived at every read and never filed, like a type is (doc 25, "Nothing is stated"), so the
//! name promotes itself as evidence arrives and two craft holding different evidence call one
//! body different things. Only the designation is stored, and it is frozen.

use super::BodyBelief;
use super::conclusion::{Class, Kind, SETTLED};
use super::sort::{Sort, equilibrium_at};
#[cfg(doc)]
use super::sort::Sorts;
use crate::sky::generate::{AU, disc::EARTH_RADIUS};
use crate::star::Star;

/// What the craft calls it: a chosen name as it stands, otherwise a descriptor and the
/// designation.
///
/// `settled` is [`Sorts::leading`] at [`SETTLED`], passed in because it is a pass over the whole
/// prior and a caller asking every frame will want to keep it.
pub fn called(belief: &BodyBelief, star: &Star, settled: Option<Sort>) -> String {
    if let Some(given) = &belief.given {
        return given.clone();
    }
    let descriptor = descriptor(belief, star, settled);
    match &belief.designation {
        Some(designation) => format!("{descriptor} {designation}"),
        None => descriptor,
    }
}

/// The most specific thing the evidence supports, least first: a moving point, then what it goes
/// round, then its size, then how warm, then what kind of world.
pub fn descriptor(belief: &BodyBelief, star: &Star, settled: Option<Sort>) -> String {
    let has_orbit = belief.semi_major_au.is_some();
    let moon = has_orbit && belief.about.is_some();
    let radius_earths = belief.radius_m.map(|(r, _)| r / EARTH_RADIUS);
    // A moon's own orbit is about its planet, so its distance says nothing of how warm it is.
    if moon {
        return moon_descriptor(radius_earths, settled);
    }
    let warmth = belief.semi_major_au.map(|(a, _)| Warmth::of(equilibrium_at(star, a * AU)));
    let prefixed = |noun: &str| match warmth {
        Some(warmth) => format!("{} {noun}", warmth.label()),
        None => noun.to_string(),
    };
    if let Some(sort) = settled {
        return match sort {
            Sort::GasGiant => prefixed("Jupiter"),
            Sort::IceGiant => prefixed("Neptune"),
            Sort::Subneptune => prefixed("Mini-Neptune"),
            Sort::Ocean => "Ocean World".into(),
            Sort::Greenhouse => "Venus-like World".into(),
            Sort::Desert => "Mars-like World".into(),
            Sort::IceWorld => "Ice World".into(),
            Sort::Barren => "Barren World".into(),
            Sort::Molten => "Lava World".into(),
        };
    }
    if let Some(r) = radius_earths {
        return match Size::of(r) {
            // Too small for how warm it is to be the interesting thing about it.
            size @ (Size::Minor | Size::Dwarf) => size.label().into(),
            size => prefixed(size.label()),
        };
    }
    if let Some(class) = transit_class(belief) {
        return prefixed(match class {
            Class::Giant => "Giant",
            Class::Rocky => "Rocky Planet",
        });
    }
    if has_orbit { "Planetoid".into() } else { "Object".into() }
}

fn moon_descriptor(radius_earths: Option<f64>, settled: Option<Sort>) -> String {
    let surface = settled.and_then(|sort| match sort {
        Sort::Ocean => Some("Ocean"),
        Sort::Greenhouse => Some("Clouded"),
        Sort::IceWorld => Some("Ice"),
        Sort::Desert | Sort::Barren => Some("Rocky"),
        Sort::Molten => Some("Lava"),
        Sort::GasGiant | Sort::IceGiant | Sort::Subneptune => None,
    });
    match (surface, radius_earths.map(Size::of)) {
        (Some(surface), _) => format!("{surface} Moon"),
        (None, Some(Size::Minor)) => "Moonlet".into(),
        (None, _) => "Moon".into(),
    }
}

/// The transit's call, when it has settled one.
fn transit_class(belief: &BodyBelief) -> Option<Class> {
    belief.kind.iter().find_map(|h| match h.kind {
        Kind::Planet { class, .. } if h.probability >= SETTLED => Some(class),
        _ => None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Warmth {
    Hot,
    Warm,
    Temperate,
    Cold,
}

impl Warmth {
    /// Zero-albedo equilibrium, so Earth is 278 K rather than the 255 K a real albedo gives.
    /// Hot is the conventional hot-Jupiter line.
    fn of(equilibrium_k: f64) -> Self {
        match equilibrium_k {
            k if k >= 1000.0 => Self::Hot,
            k if k >= 400.0 => Self::Warm,
            k if k >= 180.0 => Self::Temperate,
            _ => Self::Cold,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Hot => "Hot",
            Self::Warm => "Warm",
            Self::Temperate => "Temperate",
            Self::Cold => "Cold",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Size {
    Minor,
    Dwarf,
    Terrestrial,
    SuperEarth,
    MiniNeptune,
    Neptune,
    Jupiter,
}

impl Size {
    /// Earth radii. Minor ends near 300 km, where bodies stop pulling themselves round.
    fn of(radius_earths: f64) -> Self {
        match radius_earths {
            r if r < 0.05 => Self::Minor,
            r if r < 0.3 => Self::Dwarf,
            r if r < 1.25 => Self::Terrestrial,
            r if r < 2.0 => Self::SuperEarth,
            r if r < 4.0 => Self::MiniNeptune,
            r if r < 8.0 => Self::Neptune,
            _ => Self::Jupiter,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Minor => "Minor Body",
            Self::Dwarf => "Dwarf Planet",
            Self::Terrestrial => "Terrestrial",
            Self::SuperEarth => "Super-Earth",
            Self::MiniNeptune => "Mini-Neptune",
            Self::Neptune => "Neptune",
            Self::Jupiter => "Jupiter",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::conclusion::Hypothesis;
    use crate::knowledge::sort::tests_support::star_of;
    use crate::knowledge::{BodyId, Orientation, Placed, Subject};
    use crate::sky::StarId;

    fn sun() -> Star {
        star_of(1, 1.0).star
    }

    fn found() -> BodyBelief {
        let star = StarId::synthesize("called", 1);
        let body = BodyId::of(star, "x");
        BodyBelief {
            subject: Subject::Body { star, body },
            body,
            given: None,
            designation: Some("0-3".into()),
            kind: Vec::new(),
            period_s: None,
            semi_major_au: None,
            orientation: Orientation::Unknown,
            method: None,
            position_now: Placed::Unknown,
            radius_m: None,
            spin_s: None,
            velocity_m_s: None,
            about: None,
            colors: None,
            mass_kg: None,
            stated_by: None,
            hops: 0,
        }
    }

    fn radius(earths: f64) -> Option<(f64, f64)> {
        Some((earths * EARTH_RADIUS, 0.01 * earths * EARTH_RADIUS))
    }

    /// Each piece of evidence makes the name more specific, and the designation never moves.
    #[test]
    fn a_name_promotes_as_the_evidence_arrives() {
        let sun = sun();
        let mut belief = found();
        assert_eq!(called(&belief, &sun, None), "Object 0-3");
        belief.semi_major_au = Some((0.05, 0.001));
        assert_eq!(called(&belief, &sun, None), "Planetoid 0-3");
        belief.radius_m = radius(11.0);
        assert_eq!(called(&belief, &sun, None), "Hot Jupiter 0-3");
        belief.semi_major_au = Some((5.2, 0.01));
        assert_eq!(called(&belief, &sun, None), "Cold Jupiter 0-3");
        assert_eq!(called(&belief, &sun, Some(Sort::IceGiant)), "Cold Neptune 0-3");
    }

    #[test]
    fn a_chosen_name_stands_as_it_is() {
        let belief = BodyBelief { given: Some("Kettle".into()), ..found() };
        assert_eq!(called(&belief, &sun(), Some(Sort::Ocean)), "Kettle");
    }

    /// A moon's orbit is about its planet, so it gets no warmth from it.
    #[test]
    fn a_moon_is_called_a_moon() {
        let sun = sun();
        let about = Some(BodyId::of(StarId::synthesize("called", 1), "p"));
        let moon = BodyBelief { semi_major_au: Some((0.002, 0.0)), about, ..found() };
        assert_eq!(descriptor(&moon, &sun, None), "Moon");
        assert_eq!(descriptor(&BodyBelief { radius_m: radius(0.001), ..moon.clone() }, &sun, None), "Moonlet");
        assert_eq!(descriptor(&moon, &sun, Some(Sort::IceWorld)), "Ice Moon");
    }

    /// A transit settles rocky against giant before anything has measured a radius.
    #[test]
    fn a_transit_names_the_class() {
        let sun = sun();
        let mut belief = BodyBelief { semi_major_au: Some((1.0, 0.01)), ..found() };
        let candidate = crate::knowledge::transit::Candidate {
            period_s: 3.0e7,
            period_sigma_s: 1.0,
            epoch_s: 0.0,
            duration_s: 4.0e4,
            depth: 1.0e-4,
            depth_sigma: 1.0e-6,
            delta_chi2: 100.0,
            transits: 3,
        };
        let class = |class, probability| Hypothesis {
            probability,
            kind: Kind::Planet { class, transit: candidate },
        };
        belief.kind = vec![class(Class::Rocky, 0.6)];
        assert_eq!(descriptor(&belief, &sun, None), "Planetoid", "not settled");
        belief.kind = vec![class(Class::Rocky, 0.995)];
        assert_eq!(descriptor(&belief, &sun, None), "Temperate Rocky Planet");
    }

    #[test]
    fn small_bodies_are_not_given_a_climate() {
        let belief = BodyBelief { semi_major_au: Some((2.7, 0.01)), radius_m: radius(0.07), ..found() };
        assert_eq!(descriptor(&belief, &sun(), None), "Dwarf Planet");
    }
}
