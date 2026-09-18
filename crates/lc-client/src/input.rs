//! The only place that knows about keys.
//!
//! Input produces [`Action`] values and nothing else, so a second front end — a touch build,
//! a script, a test — needs no new plumbing, and rebinding is a change to this table.

use bevy::input::mouse::{AccumulatedMouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::input::EguiWantsInput;

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
        (KeyCode::KeyB, Action::TogglePanel(Panel::Reader)),
        (KeyCode::F4, Action::TogglePanel(Panel::Tuning)),
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
        (KeyCode::KeyN, Action::SelectNearest),
        (KeyCode::Period, Action::TimeRateUp),
        (KeyCode::Comma, Action::TimeRateDown),
        // The keyboard path for the wheel, for the same reason the arrow keys shadow the
        // mouse: it is the one that works without a pointing device.
        (KeyCode::Equal, Action::Zoom(1.0)),
        (KeyCode::Minus, Action::Zoom(-1.0)),
    ]
}

/// Turn the wheel into notches of zoom.
///
/// Pixel-precision devices report a continuous scroll rather than detents, so they are divided
/// down instead of being taken as fifty notches of zoom for one flick of a trackpad.
pub fn notches(unit: MouseScrollUnit, amount: f32) -> f64 {
    match unit {
        MouseScrollUnit::Line => amount as f64,
        MouseScrollUnit::Pixel => (amount / WHEEL_PIXELS_PER_NOTCH) as f64,
    }
}

/// The wheel, as zoom. Ignored while the interface wants it, so scrolling a panel does not
/// also fly the camera.
pub fn read_wheel(
    mut wheel: MessageReader<MouseWheel>,
    egui: Res<EguiWantsInput>,
    mut out: MessageWriter<Requested>,
) {
    let total: f64 = wheel.read().map(|w| notches(w.unit, w.y)).sum();
    if total != 0.0 && !egui.wants_any_pointer_input() {
        out.write(Requested(Action::Zoom(total)));
    }
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

/// Notches of zoom per line of wheel. A pixel-precision wheel — a trackpad — reports pixels
/// instead, and this many of them make one notch.
pub const WHEEL_PIXELS_PER_NOTCH: f32 = 50.0;

/// The button that turns the view.
pub const LOOK_BUTTON: MouseButton = MouseButton::Right;

/// Whether the view is being turned with the mouse, and the cursor is therefore pinned.
#[derive(Resource, Default)]
pub struct Looking(pub bool);

/// Pin the cursor while the look button is held, and give it back when it is released.
///
/// Locked rather than confined: a confined cursor still travels and hits the edge of the window
/// part-way through a turn, and macOS supports locking where it does not support confining.
///
/// Whether egui wants the pointer is consulted once, on the press. Asking every frame would let
/// a turn that happens to leave the cursor over a panel release itself mid-drag; asking on the
/// press is what stops a right-drag on a panel from taking the view away instead.
///
/// The component is written only on a transition. Writing it every frame is a window call every
/// frame, for a value that did not change.
pub fn grab_cursor(
    buttons: Res<ButtonInput<MouseButton>>,
    egui: Res<EguiWantsInput>,
    mut looking: ResMut<Looking>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor) = cursor.single_mut() else { return };
    let want = grab_transition(
        looking.0,
        buttons.just_pressed(LOOK_BUTTON),
        buttons.pressed(LOOK_BUTTON),
        egui.wants_any_pointer_input(),
    );
    let Some(grab) = want else { return };
    looking.0 = grab;
    cursor.grab_mode = if grab { CursorGrabMode::Locked } else { CursorGrabMode::None };
    cursor.visible = !grab;
}

/// Give the cursor back on leaving the sky, since [`grab_cursor`] only runs inside it.
pub fn release_cursor(
    mut looking: ResMut<Looking>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    looking.0 = false;
    let Ok(mut cursor) = cursor.single_mut() else { return };
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
}

/// Whether to take the cursor, give it back, or leave it alone.
///
/// Separate from the system because this is the part worth testing and `EguiWantsInput` cannot
/// be put into a chosen state from outside `bevy_egui`.
pub fn grab_transition(
    looking: bool,
    just_pressed: bool,
    held: bool,
    egui_wants_pointer: bool,
) -> Option<bool> {
    if !looking && just_pressed && !egui_wants_pointer {
        Some(true)
    } else if looking && !held {
        // Released, or the window lost focus with the button still down. Either way the cursor
        // goes back: an application holding a hidden cursor it no longer needs is one the
        // player has to force-quit.
        Some(false)
    } else {
        None
    }
}

/// Held arrow keys and the look button, as one relative turn per frame.
///
/// The right button rather than the left, so the left stays free for egui and for picking when
/// picking exists. The keyboard path is not merely a convenience: it is the one that works
/// without a pointing device at all.
pub fn look_around(
    keys: Res<ButtonInput<KeyCode>>,
    looking: Res<Looking>,
    motion: Res<AccumulatedMouseMotion>,
    time: Res<Time>,
    state: Res<crate::app::Ui>,
    mut out: MessageWriter<Requested>,
) {
    let mut yaw = 0.0;
    let mut pitch = 0.0;

    // The arrows belong to the book while one is open. The mouse still turns the view, because
    // nothing about reading stops the ship.
    let turning = !state.is_open(Panel::Reader);
    let step = LOOK_STEP * time.delta_secs_f64() * 60.0;
    for (key, (y, p)) in held_bindings() {
        if turning && keys.pressed(key) {
            yaw += y * step;
            pitch += p * step;
        }
    }
    if looking.0 {
        let d = motion.delta;
        yaw -= d.x as f64 * MOUSE_SENSITIVITY;
        pitch -= d.y as f64 * MOUSE_SENSITIVITY;
    }

    if yaw != 0.0 || pitch != 0.0 {
        out.write(Requested(Action::Look { yaw, pitch }));
    }
}

/// Turn key presses into requests.
/// What the keyboard is doing, which is not the same thing in a book as in a cockpit.
///
/// Arrow keys turn the view and the reader needs them to turn pages, so the table is chosen by
/// mode rather than merged. egui cannot decide this: `EguiWantsInput` reports keyboard interest
/// only when a *text field* has focus, and a page of prose has none.
pub fn reading_bindings() -> Vec<(KeyCode, Action)> {
    vec![
        (KeyCode::Escape, Action::CloseBook),
        (KeyCode::Space, Action::TurnPage(1)),
        (KeyCode::ArrowRight, Action::TurnPage(1)),
        (KeyCode::PageDown, Action::TurnPage(1)),
        (KeyCode::Backspace, Action::TurnPage(-1)),
        (KeyCode::ArrowLeft, Action::TurnPage(-1)),
        (KeyCode::PageUp, Action::TurnPage(-1)),
        (KeyCode::KeyC, Action::ToggleContents),
        // Out of the book and back to the shelf, which is the same key that opened the device
        // and one step of the same hierarchy `Escape` walks.
        (KeyCode::KeyB, Action::CloseBook),
    ]
}

pub fn read_keys(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<crate::app::Ui>,
    egui: Res<EguiWantsInput>,
    mut out: MessageWriter<Requested>,
) {
    // A field being typed into owns the keyboard, all of it. The shelf's filter is the only one
    // in this client, and without this, typing the name of a book closes the window on `b` and
    // turns a page on the space bar.
    if egui.wants_any_keyboard_input() {
        return;
    }
    let reading = state.is_open(Panel::Reader);
    let table = if reading { reading_bindings() } else { bindings() };
    for (key, action) in table {
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
    fn no_reading_key_is_bound_twice() {
        let b = reading_bindings();
        let mut keys: Vec<KeyCode> = b.iter().map(|(k, _)| *k).collect();
        let before = keys.len();
        keys.sort_by_key(|k| format!("{k:?}"));
        keys.dedup();
        assert_eq!(keys.len(), before, "a key turns two pages at once");
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

    /// A headless app with the two look systems and a window carrying cursor options.
    fn harness() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<Requested>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<EguiWantsInput>()
            .init_resource::<Looking>()
            // The arrows belong to the view unless a book is open, so the systems under test
            // need to be able to ask which it is.
            .insert_resource(crate::app::Ui(crate::ui::UiState::default()))
            .add_systems(Update, (grab_cursor, look_around).chain());
        let window = app
            .world_mut()
            .spawn((Window::default(), CursorOptions::default(), PrimaryWindow))
            .id();
        (app, window)
    }

    fn press(app: &mut App) {
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(LOOK_BUTTON);
    }

    fn release(app: &mut App) {
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().release(LOOK_BUTTON);
    }

    fn cursor(app: &App, window: Entity) -> CursorOptions {
        app.world().entity(window).get::<CursorOptions>().unwrap().clone()
    }

    #[test]
    fn holding_the_look_button_pins_the_cursor_and_releasing_gives_it_back() {
        let (mut app, window) = harness();
        assert_eq!(cursor(&app, window).grab_mode, CursorGrabMode::None);

        press(&mut app);
        app.update();
        assert_eq!(cursor(&app, window).grab_mode, CursorGrabMode::Locked);
        assert!(!cursor(&app, window).visible);

        release(&mut app);
        app.update();
        assert_eq!(cursor(&app, window).grab_mode, CursorGrabMode::None);
        assert!(cursor(&app, window).visible, "a hidden cursor nobody can free is a force-quit");
    }

    /// The failure this guards: losing focus mid-drag leaves the button never released, and a
    /// client that keeps the cursor in that state cannot be clicked out of.
    #[test]
    fn the_cursor_comes_back_even_if_the_release_is_never_seen() {
        let (mut app, window) = harness();
        press(&mut app);
        app.update();
        // Not a release -- the input resource simply stops reporting the button as held.
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().reset_all();
        app.update();
        assert_eq!(cursor(&app, window).grab_mode, CursorGrabMode::None);
        assert!(cursor(&app, window).visible);
    }

    #[test]
    fn a_right_drag_on_a_panel_does_not_take_the_view() {
        assert_eq!(grab_transition(false, true, true, true), None, "egui had the pointer");
        assert_eq!(grab_transition(false, true, true, false), Some(true));
    }

    /// Asked once on the press, not every frame: a turn that happens to leave the cursor over a
    /// panel must not release itself part-way through.
    #[test]
    fn a_turn_already_under_way_is_not_interrupted_by_a_panel() {
        assert_eq!(grab_transition(true, false, true, true), None, "must stay grabbed");
        assert_eq!(grab_transition(true, false, false, true), Some(false), "and release on let go");
    }

    #[test]
    fn a_steady_state_writes_nothing() {
        assert_eq!(grab_transition(false, false, false, false), None);
        assert_eq!(grab_transition(true, false, true, false), None);
    }

    #[test]
    fn the_mouse_only_turns_the_view_while_it_is_held() {
        let (mut app, _) = harness();
        let nudge = |app: &mut App| {
            app.world_mut().resource_mut::<AccumulatedMouseMotion>().delta = Vec2::new(10.0, 0.0);
            app.update();
            app.world_mut().resource_mut::<Messages<Requested>>().drain().count()
        };
        assert_eq!(nudge(&mut app), 0, "a loose mouse must not turn the view");
        press(&mut app);
        assert_eq!(nudge(&mut app), 1, "a held one must");
    }

    /// The keyboard path does not depend on a pointing device being present at all.
    ///
    /// Two updates, not one: the keyboard turn is scaled by the frame time and the first frame
    /// of any app has a delta of zero.
    #[test]
    fn the_arrow_keys_turn_the_view_without_the_mouse() {
        let (mut app, _) = harness();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::ArrowLeft);
        app.update();
        app.world_mut().resource_mut::<Messages<Requested>>().clear();
        app.update();
        assert_eq!(app.world_mut().resource_mut::<Messages<Requested>>().drain().count(), 1);
    }

    #[test]
    fn a_book_takes_the_arrow_keys_from_the_view() {
        let (mut app, _) = harness();
        app.world_mut().resource_mut::<crate::app::Ui>().open(Panel::Reader);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::ArrowLeft);
        app.update();
        app.world_mut().resource_mut::<Messages<Requested>>().clear();
        app.update();
        assert_eq!(
            app.world_mut().resource_mut::<Messages<Requested>>().drain().count(),
            0,
            "the view turned while a page was being read"
        );
    }
}
