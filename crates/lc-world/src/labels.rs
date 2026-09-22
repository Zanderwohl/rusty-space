//! What a craft calls the bodies of the system it is in.
//!
//! A body's own name is only ever a key. A body is called what this craft named it, or else
//! what the designation rule makes of its place. See `lightcone/docs/23-factions.md`, "What
//! things are called".

use std::collections::BTreeMap;

use crate::knowledge::BodyId;
use crate::knowledge::names::{SPACING, planet_letter};
use crate::navigation::{Kind, Target, roman};
use crate::sky::generate::AU;
use crate::system::LocalSystem;

/// Labels for a local system's bodies, by the key the rest of the code targets them by.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Labels {
    by_key: BTreeMap<String, String>,
}

impl Labels {
    /// A key with no label is a body this pass did not see; it is never called by its key.
    pub fn of(&self, key: &str) -> String {
        self.by_key.get(key).cloned().unwrap_or_else(|| "unidentified body".into())
    }
}

/// `star` is what this craft calls the system's star.
pub fn label(system: &LocalSystem, star: &str, named: impl Fn(BodyId) -> Option<String>) -> Labels {
    let mut by_key = BTreeMap::new();
    let luminosity_solar = system.star_luminosity_w() / em_spectra::stellar::SOLAR_LUMINOSITY;
    // The next planet letter is placed against these.
    let mut placed: Vec<(String, f64)> = Vec::new();
    // The label at each depth of the walk, and how many children each has had.
    let mut parents: Vec<(String, usize)> = vec![(star.to_string(), 0)];
    by_key.insert(system.sim().name(system.primary()).to_string(), star.to_string());
    for entry in system.inventory() {
        let Target::Body(key) = &entry.target else { continue };
        if entry.depth == 0 {
            continue;
        }
        parents.truncate(entry.depth);
        let rank = match parents.last_mut() {
            Some(parent) => {
                parent.1 += 1;
                parent.1
            }
            None => 1,
        };
        let parent = parents.last().map_or(star.to_string(), |p| p.0.clone());
        let label = match named(BodyId::of(system.star, key)) {
            Some(name) => name,
            None if entry.depth == 1 && entry.kind == Kind::Planet => {
                let a_au = entry.orbit_radius_m / AU;
                let letter = planet_letter(&placed, a_au, luminosity_solar, SPACING);
                placed.push((letter.clone(), a_au));
                format!("{star} {letter}")
            }
            None => format!("{parent} {}", roman(rank)),
        };
        by_key.insert(key.clone(), label.clone());
        parents.push((label, 0));
    }
    Labels { by_key }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{AuthoredStars, StarProvider};

    fn system() -> LocalSystem {
        // The third authored star is the one whose generated system has planets.
        let stars = AuthoredStars::sample();
        LocalSystem::for_star(&stars.stars()[2]).expect("a generated system")
    }

    /// No generator name reaches a label, and a body the craft named is called that.
    #[test]
    fn bodies_are_called_what_this_craft_calls_them() {
        let system = system();
        let labels = label(&system, "Hearth", |_| None);
        let keys: Vec<&String> = labels.by_key.keys().collect();
        assert!(keys.len() > 1);
        for (key, name) in &labels.by_key {
            assert!(name.starts_with("Hearth"), "{key} is called {name}");
            assert!(!name.contains("Authored"), "a generator's name reached a label: {name}");
        }
        let (key, _) = labels.by_key.iter().find(|(_, name)| name.as_str() != "Hearth").unwrap();
        let named = BodyId::of(system.star, key);
        let renamed = label(&system, "Hearth", |id| (id == named).then(|| "Spout".to_string()));
        assert_eq!(renamed.of(key), "Spout");
    }
}
