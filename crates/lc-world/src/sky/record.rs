//! The minimum a catalogue has to supply, and the one place it becomes a [`CatalogueStar`].
//!
//! Everything else about a star — radius, temperature, mu, mass, metallicity — is derived,
//! and it is derived here. Two importers deriving it separately is two chances to disagree,
//! and the disagreement would be silent: both would produce a plausible star.

use em_spectra::{colour_index, stellar};
use glam::DVec3;

use super::{CatalogueStar, Component, Provenance, StarId};
use crate::sky::metallicity;
use crate::star::Star;

/// Solar limb darkening, applied to every star.
///
/// A real coefficient pair depends on temperature and band. Until the photometry asks for
/// that difference, one pair is honest and two would be a decoration.
const LIMB_DARKENING: (f64, f64) = (0.4, 0.26);

/// One row of a catalogue, reduced to what cannot be recomputed.
#[derive(Clone, Debug, PartialEq)]
pub struct StarRecord {
    /// The catalogue's own key. Provenance, never an identity.
    pub key: u64,
    pub name: Option<String>,
    /// Ecliptic, light-years.
    pub position_ly: DVec3,
    /// Ecliptic, m/s.
    pub velocity: DVec3,
    /// `B-V`.
    pub colour_index: f64,
    pub luminosity_solar: f64,
    /// 1 for a single star or the primary of a multiple.
    pub component_index: u8,
    /// The catalogue's key for the primary of this star's system; `None` for a single.
    pub group: Option<u64>,
}

impl StarRecord {
    /// `None` when the numbers do not describe a star.
    ///
    /// Returning an option rather than filtering at the call site means both importers reject
    /// the same rows for the same reasons.
    pub fn assemble(&self, source: &str) -> Option<CatalogueStar> {
        if !colour_index::bv_is_valid(self.colour_index) || self.luminosity_solar <= 0.0 {
            return None;
        }
        let teff = colour_index::teff_from_bv(self.colour_index);
        let luminosity_w = self.luminosity_solar * stellar::SOLAR_LUMINOSITY;
        let radius = stellar::radius_from_luminosity(luminosity_w, teff);
        if !(radius.is_finite() && radius > 0.0) {
            return None;
        }
        let mass_solar = stellar::main_sequence_mass_solar(self.luminosity_solar);
        let id = StarId::synthesise(source, self.key);

        Some(CatalogueStar {
            id,
            provenance: Provenance { source: source.into(), key: self.key },
            name: self.name.clone(),
            position_ly: self.position_ly,
            velocity: self.velocity,
            star: Star {
                radius_m: radius,
                teff_k: teff,
                mu: stellar::mu_from_mass_solar(mass_solar),
                limb_darkening: LIMB_DARKENING,
            },
            luminosity_solar: self.luminosity_solar,
            mass_solar,
            metallicity: metallicity::from_speed(self.velocity.length(), id.get()),
            component: Component { index: self.component_index, group: self.group },
        })
    }
}

/// Clears the group of any star that turns out to be alone in it.
///
/// Catalogues name a primary on every row, their own included, so grouping is provisional
/// until the whole set has been read. A group of one is not a multiple.
///
/// This runs on assembled stars, not on records, and the order matters: a record whose
/// partner is rejected by [`StarRecord::assemble`] would still be counted as a pair if the
/// pruning happened first, and would then be the only member of a group it kept.
pub fn prune_singleton_groups(stars: &mut [CatalogueStar]) {
    let mut counts: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
    for s in stars.iter() {
        if let Some(g) = s.component.group {
            *counts.entry(g).or_default() += 1;
        }
    }
    for s in stars.iter_mut() {
        if s.component.group.is_some_and(|g| counts[&g] < 2) {
            s.component.group = None;
        }
    }
}

/// Assembles a whole catalogue: derives every star, drops what will not derive, and resolves
/// grouping over the survivors. Every importer goes through this, which is what makes the CSV
/// and the packed chunk the same catalogue.
pub fn assemble_all(source: &str, records: &[StarRecord]) -> (Vec<CatalogueStar>, usize) {
    let mut stars = Vec::with_capacity(records.len());
    let mut skipped = 0;
    for record in records {
        match record.assemble(source) {
            Some(star) => stars.push(star),
            None => skipped += 1,
        }
    }
    prune_singleton_groups(&mut stars);
    (stars, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sol() -> StarRecord {
        StarRecord {
            key: 0,
            name: Some("Sol".into()),
            position_ly: DVec3::ZERO,
            velocity: DVec3::ZERO,
            colour_index: 0.656,
            luminosity_solar: 1.0,
            component_index: 1,
            group: None,
        }
    }

    #[test]
    fn a_solar_record_assembles_to_a_solar_star() {
        let s = sol().assemble("test").expect("Sol assembles");
        assert!((s.star.teff_k - 5772.0).abs() < 40.0, "Teff {}", s.star.teff_k);
        assert!((s.star.radius_m / em_spectra::stellar::SOLAR_RADIUS - 1.0).abs() < 0.05);
        assert!((s.mass_solar - 1.0).abs() < 0.05);
    }

    #[test]
    fn unusable_numbers_are_rejected_rather_than_fudged() {
        let mut r = sol();
        r.luminosity_solar = 0.0;
        assert!(r.assemble("test").is_none());
        let mut r = sol();
        r.colour_index = 99.0;
        assert!(r.assemble("test").is_none());
    }

    #[test]
    fn the_id_follows_the_source_and_never_the_key() {
        let r = sol();
        let a = r.assemble("one").unwrap();
        let b = r.assemble("two").unwrap();
        assert_ne!(a.id, b.id, "the same key in two catalogues is two stars");
        assert_ne!(a.id.get(), r.key);
    }

    #[test]
    fn a_group_of_one_is_not_a_multiple() {
        let mut rs = vec![sol(), sol(), sol()];
        rs[0].group = Some(7);
        rs[1].group = Some(7);
        rs[2].group = Some(9);
        let (stars, skipped) = assemble_all("test", &rs);
        assert_eq!(skipped, 0);
        assert_eq!(stars[0].component.group, Some(7));
        assert_eq!(stars[1].component.group, Some(7));
        assert_eq!(stars[2].component.group, None, "a lone member keeps no group");
    }

    #[test]
    fn a_partner_that_does_not_assemble_leaves_no_group_behind() {
        let mut rs = vec![sol(), sol()];
        rs[0].group = Some(7);
        rs[1].group = Some(7);
        // The second star has an unusable luminosity, so only one member survives.
        rs[1].luminosity_solar = 0.0;
        let (stars, skipped) = assemble_all("test", &rs);
        assert_eq!(skipped, 1);
        assert_eq!(stars.len(), 1);
        assert_eq!(stars[0].component.group, None, "grouping must follow the survivors");
    }
}
