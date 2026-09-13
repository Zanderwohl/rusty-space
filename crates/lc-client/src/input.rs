//! The only place that knows about keys.
//!
//! Input produces [`Action`] values and nothing else, so a second front end — a touch build,
//! a script, a test — needs no new plumbing, and rebinding is a change to this table.

use bevy::prelude::*;

use crate::action::Action;
use crate::ui::Panel;

/// An action asked for by something. Carried as a message so every source looks the same.
#[derive(Message, Clone, Debug)]
pub struct Requested(pub Action);

/// The default bindings.
pub fn bindings() -> Vec<(KeyCode, Action)> {
    vec![
        (KeyCode::Escape, Action::CloseTopPanel),
        (KeyCode::KeyT, Action::TogglePanel(Panel::Telescope)),
        (KeyCode::KeyY, Action::TogglePanel(Panel::System)),
        (KeyCode::F3, Action::TogglePanel(Panel::Debug)),
        (KeyCode::Digit1, Action::SetBandPreset(0)),
        (KeyCode::Digit2, Action::SetBandPreset(1)),
        (KeyCode::Digit3, Action::SetBandPreset(2)),
        (KeyCode::Digit4, Action::SetBandPreset(3)),
        (KeyCode::Digit5, Action::SetBandPreset(4)),
        (KeyCode::Digit6, Action::SetBandPreset(5)),
        (KeyCode::BracketRight, Action::ExposureUp),
        (KeyCode::BracketLeft, Action::ExposureDown),
        (KeyCode::Backslash, Action::ExposureAuto),
    ]
}

/// Turn key presses into requests.
pub fn read_keys(keys: Res<ButtonInput<KeyCode>>, mut out: MessageWriter<Requested>) {
    for (key, action) in bindings() {
        if keys.just_pressed(key) {
            out.write(Requested(action));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_is_bound_twice() {
        let b = bindings();
        let mut keys: Vec<KeyCode> = b.iter().map(|(k, _)| *k).collect();
        let before = keys.len();
        keys.sort_by_key(|k| format!("{k:?}"));
        keys.dedup();
        assert_eq!(keys.len(), before, "a key is bound to two actions");
    }

    #[test]
    fn the_band_presets_are_all_reachable() {
        let bound: Vec<usize> = bindings()
            .iter()
            .filter_map(|(_, a)| match a {
                Action::SetBandPreset(i) => Some(*i),
                _ => None,
            })
            .collect();
        for i in 0..em_spectra::presets::all().len() {
            assert!(bound.contains(&i), "preset {i} has no key");
        }
    }
}
