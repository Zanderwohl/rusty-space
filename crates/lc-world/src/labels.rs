//! What a craft calls the bodies of the system it is in, by the key the rest of the code
//! targets them by.
//!
//! A body's own name is only ever a key, and the inventory is only ever a list of keys: what a
//! body is called comes from what the craft holds about it, never from what the generator says
//! it is. See `lightcone/docs/23-factions.md`, "What things are called".

use std::collections::BTreeMap;

use crate::knowledge::BodyId;
use crate::navigation::Target;
use crate::system::LocalSystem;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Labels {
    by_key: BTreeMap<String, String>,
}

impl Labels {
    /// A key with no label is a body this craft has not found; it is never called by its key.
    pub fn of(&self, key: &str) -> String {
        self.by_key.get(key).cloned().unwrap_or_else(|| "unidentified body".into())
    }
}

/// `star` is what this craft calls the system's star, and `called` what it calls a body it
/// holds — `None` for one it does not.
pub fn label(system: &LocalSystem, star: &str, called: impl Fn(BodyId) -> Option<String>) -> Labels {
    let mut by_key = BTreeMap::new();
    by_key.insert(system.sim().name(system.primary()).to_string(), star.to_string());
    for entry in system.inventory() {
        let Target::Body(key) = &entry.target else { continue };
        if entry.depth == 0 {
            continue;
        }
        if let Some(label) = called(BodyId::of(system.star, key)) {
            by_key.insert(key.clone(), label);
        }
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

    /// A body is called what the craft calls it, and one it has not found is not called
    /// anything the generator knows.
    #[test]
    fn bodies_are_called_what_this_craft_calls_them() {
        let system = system();
        let keys: Vec<String> = system
            .inventory()
            .iter()
            .filter(|e| e.depth > 0)
            .filter_map(|e| match &e.target {
                Target::Body(key) => Some(key.clone()),
                _ => None,
            })
            .collect();
        let (found, missed) = (&keys[0], &keys[1]);
        let spotted = BodyId::of(system.star, found);
        let labels = label(&system, "Hearth", |id| (id == spotted).then(|| "Spout".to_string()));
        assert_eq!(labels.of(found), "Spout");
        assert_eq!(labels.of(missed), "unidentified body", "truth named a body nobody found");
        assert_eq!(labels.of(system.sim().name(system.primary())), "Hearth");
    }
}
