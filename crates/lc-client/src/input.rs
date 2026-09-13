//! The only place that knows about keys.
//!
//! Input produces [`Action`] values and nothing else, so a second front end — a touch build,
//! a script, a test — needs no new plumbing, and rebinding is a change to this table.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;

use crate::action::{Action, LOOK_STEP};
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
        (KeyCode::KeyF, Action::TogglePanel(Panel::Flight)),
        (KeyCode::Digit1, Action::SetBandPreset(0)),
        (KeyCode::Digit2, Action::SetBandPreset(1)),
        (KeyCode::Digit3, Action::SetBandPreset(2)),
        (KeyCode::Digit4, Action::SetBandPreset(3)),
        (KeyCode::Digit5, Action::SetBandPreset(4)),
        (KeyCode::Digit6, Action::SetBandPreset(5)),
        (KeyCode::BracketRight, Action::ExposureUp),
        (KeyCode::BracketLeft, Action::ExposureDown),
        (KeyCode::Backslash, Action::ExposureAuto),
        (KeyCode::KeyG, Action::FlyTo(None)),
        (KeyCode::KeyX, Action::AbortFlight),
        (KeyCode::KeyL, Action::LookAtSelected),
    ]
}

/// Keys held rather than pressed. Look is continuous, so it scales with the frame.
pub fn held_bindings() -> Vec<(KeyCode, (f64, f64))> {
    vec![
        (KeyCode::ArrowLeft, (1.0, 0.0)),
        (KeyCode::ArrowRight, (-1.0, 0.0)),
        (KeyCode::ArrowUp, (0.0, 1.0)),
        (KeyCode::ArrowDown, (0.0, -1.0)),
    ]
}

/// Radians of look per pixel of mouse movement.
pub const MOUSE_SENSITIVITY: f64 = 0.003;

/// Held arrow keys and a dragged right mouse button, as one relative turn per frame.
///
/// The right button rather than the left, and drag rather than pointer capture, so egui keeps
/// a usable cursor: every control in this client is reachable by mouse and nothing grabs it.
pub fn look_around(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    time: Res<Time>,
    mut out: MessageWriter<Requested>,
) {
    let mut yaw = 0.0;
    let mut pitch = 0.0;

    let step = LOOK_STEP * time.delta_secs_f64() * 60.0;
    for (key, (y, p)) in held_bindings() {
        if keys.pressed(key) {
            yaw += y * step;
            pitch += p * step;
        }
    }
    if buttons.pressed(MouseButton::Right) {
        let d = motion.delta;
        yaw -= d.x as f64 * MOUSE_SENSITIVITY;
        pitch -= d.y as f64 * MOUSE_SENSITIVITY;
    }

    if yaw != 0.0 || pitch != 0.0 {
        out.write(Requested(Action::Look { yaw, pitch }));
    }
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
    fn held_and_pressed_bindings_do_not_overlap() {
        let pressed: Vec<KeyCode> = bindings().iter().map(|(k, _)| *k).collect();
        for (key, _) in held_bindings() {
            assert!(!pressed.contains(&key), "{key:?} is both held and pressed");
        }
    }

    #[test]
    fn look_can_be_driven_in_all_four_directions() {
        let mut seen = (false, false, false, false);
        for (_, (y, p)) in held_bindings() {
            seen.0 |= y > 0.0;
            seen.1 |= y < 0.0;
            seen.2 |= p > 0.0;
            seen.3 |= p < 0.0;
        }
        assert_eq!(seen, (true, true, true, true), "a direction has no key");
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
