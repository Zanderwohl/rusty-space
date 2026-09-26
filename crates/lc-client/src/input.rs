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
        (KeyCode::KeyR, Action::TogglePanel(Panel::Refit)),
        (KeyCode::F5, Action::TogglePanel(Panel::DevActions)),
        // `R` is the refit window's. `C` for comms, which is what this is.
        (KeyCode::KeyC, Action::TogglePanel(Panel::Chat)),
        // Opens and never closes: once the field has the keyboard, a slash is a slash.
        (KeyCode::Slash, Action::OpenPanel(Panel::Console)),
        (KeyCode::KeyM, Action::ToggleView),
        // For hangar, where KSP builds its planes.
        (KeyCode::KeyH, Action::ToggleForm),
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

/// Bindings taken with Shift held, over the same key's plain one.
pub fn shifted_bindings() -> Vec<(KeyCode, Action)> {
    vec![
        // `|`: `\` is automatic exposure.
        (KeyCode::Backslash, Action::ToggleBeautyShots),
    ]
}

/// The letter that does `action` unshifted, for a button to underline.
pub fn letter_for(action: &Action) -> Option<char> {
    let (key, _) = bindings().into_iter().find(|(_, a)| a == action)?;
    let name = format!("{key:?}");
    let letter = name.strip_prefix("Key")?;
    let mut chars = letter.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// `table` with the shifted bindings laid over it while Shift is held.
pub fn with_shift(mut table: Vec<(KeyCode, Action)>, shift: bool) -> Vec<(KeyCode, Action)> {
    if !shift {
        return table;
    }
    let overlay = shifted_bindings();
    table.retain(|(key, _)| !overlay.iter().any(|(claimed, _)| claimed == key));
    table.extend(overlay);
    table
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
/// also fly the camera. In the editor, Shift turns it into the slide fore and aft.
pub fn read_wheel(
    mut wheel: MessageReader<MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    egui: Res<EguiWantsInput>,
    controls: em_ui::Controls,
    state: Res<crate::app::Ui>,
    mut out: MessageWriter<Requested>,
) {
    let total: f64 = wheel.read().map(|w| notches(w.unit, w.y)).sum();
    if total == 0.0 || egui.wants_any_pointer_input() || controls.under_pointer() {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    out.write(Requested(wheel_action(state.view, shift, total)));
}

/// What `notches` of wheel do, in a view, with Shift held or not.
pub fn wheel_action(view: crate::ui::ViewMode, shift: bool, notches: f64) -> Action {
    match (view, shift) {
        (crate::ui::ViewMode::Form, true) => Action::SlideForm(notches * crate::form_view::SLIDE_PER_NOTCH),
        _ => Action::Zoom(notches),
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

/// Keys held to slide the editor's focus, toward the nose or the stern. KSP's, and unbound
/// elsewhere: the reader takes them only with a book open, when the slide stands down.
pub fn slide_bindings() -> Vec<(KeyCode, f64)> {
    vec![(KeyCode::PageUp, 1.0), (KeyCode::PageDown, -1.0)]
}

/// Radians of look per pixel of mouse movement.
pub const MOUSE_SENSITIVITY: f64 = 0.003;

/// A pointer movement as a turn of the view, radians of yaw and pitch.
///
/// Shared, because the sky reads it off a locked cursor and the map's corner square off an
/// ordinary drag: the same movement has to mean the same turn on both.
pub fn look_from(delta: Vec2) -> (f64, f64) {
    (-delta.x as f64 * MOUSE_SENSITIVITY, -delta.y as f64 * MOUSE_SENSITIVITY)
}

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
    controls: em_ui::Controls,
    mut looking: ResMut<Looking>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let want = grab_transition(
        looking.0,
        buttons.just_pressed(LOOK_BUTTON),
        buttons.pressed(LOOK_BUTTON),
        egui.wants_any_pointer_input() || controls.under_pointer(),
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
    egui: Res<EguiWantsInput>,
    looking: Res<Looking>,
    motion: Res<AccumulatedMouseMotion>,
    time: Res<Time>,
    state: Res<crate::app::Ui>,
    typing: em_ui::Typing,
    mut out: MessageWriter<Requested>,
) {
    let mut yaw = 0.0;
    let mut pitch = 0.0;

    // The arrows only, and not when something else is using them. An arrow key in a text field
    // moves the cursor, and turning the ship as well would make going back to fix a typo swing
    // the whole view; an arrow key in an open book turns its page. The mouse below is unaffected
    // by either, because holding the look button is not something a field or a page can mean.
    let step = LOOK_STEP * time.delta_secs_f64() * 60.0;
    let paging = state.reading.book.is_some();
    if !egui.wants_any_keyboard_input() && !typing.active() && !paging {
        for (key, (y, p)) in held_bindings() {
            if keys.pressed(key) {
                yaw += y * step;
                pitch += p * step;
            }
        }
    }
    if looking.0 {
        let (d_yaw, d_pitch) = look_from(motion.delta);
        yaw += d_yaw;
        pitch += d_pitch;
    }

    if yaw != 0.0 || pitch != 0.0 {
        // The same turn of the same controls, of whichever camera is the view.
        out.write(Requested(match state.view {
            crate::ui::ViewMode::Form => crate::form_view::orbit_from(yaw, pitch),
            _ => Action::Look { yaw, pitch },
        }));
    }
}

/// Turn key presses into requests.
/// The keys the reader claims, and **only** those.
///
/// An overlay rather than a second table. Swapping tables was the obvious shape and the wrong
/// one: it took every other key with it, so opening a book turned off the telescope, the system
/// window and the time controls — and every panel added afterwards would have had to be
/// remembered here to keep working. What the reader needs is the paging keys and the way back to
/// the shelf; everything it does not name falls through to the cockpit.
///
/// **Nothing mnemonic is claimed.** The contents list wanted `C` and `C` is Communications, so
/// the contents has a button and no key rather than a key that shadows a window. `Escape` is
/// left alone for the same reason: it closes the top panel wherever you are.
///
/// `book` is whether one is actually open. With the shelf showing there is nothing to page and
/// nothing to leave, so the table is exactly the cockpit's.
pub fn reading_bindings(book: bool) -> Vec<(KeyCode, Action)> {
    if !book {
        return Vec::new();
    }
    vec![
        (KeyCode::ArrowRight, Action::TurnPage(1)),
        (KeyCode::PageDown, Action::TurnPage(1)),
        (KeyCode::ArrowLeft, Action::TurnPage(-1)),
        (KeyCode::PageUp, Action::TurnPage(-1)),
        // Out of the book and back to the shelf, which is the key that opened the device.
        (KeyCode::KeyB, Action::CloseBook),
    ]
}

/// The bindings in force: the cockpit's, with the reader's laid over them.
pub fn bindings_in_force(reading: bool, book: bool) -> Vec<(KeyCode, Action)> {
    let mut table = bindings();
    if !reading {
        return table;
    }
    let overlay = reading_bindings(book);
    table.retain(|(key, _)| !overlay.iter().any(|(claimed, _)| claimed == key));
    table.extend(overlay);
    table
}

/// **Silent while the interface is taking text**, in egui or in an `em_ui` field. Every binding
/// here is a bare letter, so a player typing a message into the radio window would otherwise open
/// the telescope, cut the drive and fly somewhere, one keystroke at a time — and typing the name
/// of a book into the shelf's filter would put the book away on `b`.
pub fn read_keys(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<crate::app::Ui>,
    egui: Res<EguiWantsInput>,
    typing: em_ui::Typing,
    mut out: MessageWriter<Requested>,
) {
    if egui.wants_any_keyboard_input() || typing.active() {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let table = with_shift(
        bindings_in_force(state.is_open(Panel::Reader), state.reading.book.is_some()),
        shift,
    );
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
    fn shift_backslash_is_beauty_shots_and_backslash_alone_is_still_exposure() {
        let acts = |table: &[(KeyCode, Action)], key: KeyCode| {
            table.iter().find(|(k, _)| *k == key).map(|(_, a)| a.clone())
        };
        let plain = with_shift(bindings(), false);
        let shifted = with_shift(bindings(), true);
        assert_eq!(acts(&plain, KeyCode::Backslash), Some(Action::ExposureAuto));
        assert_eq!(acts(&shifted, KeyCode::Backslash), Some(Action::ToggleBeautyShots));
        let keys: Vec<KeyCode> = shifted.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys.iter().filter(|k| **k == KeyCode::Backslash).count(), 1);
        assert_eq!(acts(&shifted, KeyCode::KeyT), acts(&plain, KeyCode::KeyT), "the rest are as they were");
    }

    #[test]
    fn a_letter_is_found_only_for_a_plain_letter_key() {
        assert_eq!(letter_for(&Action::TogglePanel(Panel::Telescope)), Some('T'));
        assert_eq!(letter_for(&Action::ToggleView), Some('M'));
        assert_eq!(letter_for(&Action::TogglePanel(Panel::Debug)), None, "F3 is not a letter");
        assert_eq!(letter_for(&Action::ToggleBeautyShots), None, "shifted, and not a letter");
    }

    #[test]
    fn no_reading_key_is_bound_twice() {
        let b = reading_bindings(true);
        let mut keys: Vec<KeyCode> = b.iter().map(|(k, _)| *k).collect();
        let before = keys.len();
        keys.sort_by_key(|k| format!("{k:?}"));
        keys.dedup();
        assert_eq!(keys.len(), before, "a key turns two pages at once");
    }

    #[test]
    fn a_book_claims_the_paging_keys_and_leaves_the_rest_alone() {
        let flying = bindings_in_force(false, false);
        let reading = bindings_in_force(true, true);
        let acts = |table: &[(KeyCode, Action)], key: KeyCode| {
            table.iter().find(|(k, _)| *k == key).map(|(_, a)| a.clone())
        };

        // The keys a book needs are the book's.
        assert_eq!(acts(&reading, KeyCode::ArrowRight), Some(Action::TurnPage(1)));
        assert_eq!(acts(&reading, KeyCode::KeyB), Some(Action::CloseBook));
        // And `Escape` is not one of them: it closes the top panel here as it does anywhere.
        assert_eq!(acts(&reading, KeyCode::Escape), acts(&flying, KeyCode::Escape));

        // Every other panel still opens while one is being read. This is the whole point: a
        // panel added later must not have to be remembered in two places to keep working.
        for key in [KeyCode::KeyT, KeyCode::KeyY, KeyCode::F3, KeyCode::KeyF, KeyCode::Digit1] {
            assert_eq!(acts(&reading, key), acts(&flying, key), "{key:?} was eaten by the reader");
        }
        // Precisely: the only keys that behave differently are the ones the reader names.
        let claimed: Vec<KeyCode> = reading_bindings(true).iter().map(|(k, _)| *k).collect();
        for (key, action) in &flying {
            if claimed.contains(key) {
                continue;
            }
            assert_eq!(acts(&reading, *key).as_ref(), Some(action), "{key:?} changed meaning");
        }
        for (key, _) in &reading {
            assert!(
                claimed.contains(key) || flying.iter().any(|(k, _)| k == key),
                "{key:?} appeared from nowhere",
            );
        }
    }

    #[test]
    fn the_shelf_claims_nothing_at_all() {
        // No pages to turn and no book to leave, so every key means what it meant before.
        assert_eq!(bindings_in_force(true, false), bindings_in_force(false, false));
    }

    #[test]
    fn nothing_in_force_is_bound_twice() {
        for (reading, book) in [(false, false), (true, false), (true, true)] {
            let table = bindings_in_force(reading, book);
            let mut keys: Vec<KeyCode> = table.iter().map(|(k, _)| *k).collect();
            let before = keys.len();
            keys.sort_by_key(|k| format!("{k:?}"));
            keys.dedup();
            assert_eq!(keys.len(), before, "a key is bound twice at ({reading}, {book})");
        }
    }

    #[test]
    fn held_and_pressed_bindings_do_not_overlap() {
        let pressed: Vec<KeyCode> = bindings().iter().map(|(k, _)| *k).collect();
        for (key, _) in held_bindings() {
            assert!(!pressed.contains(&key), "{key:?} is both held and pressed");
        }
    }

    /// The slide's keys mean nothing else in force, and the one place they do — a book's pages —
    /// is where the slide stands down.
    #[test]
    fn the_slide_keys_are_free() {
        let held: Vec<KeyCode> = held_bindings().iter().map(|(k, _)| *k).collect();
        for (key, _) in slide_bindings() {
            assert!(!held.contains(&key), "{key:?} also turns the view");
            for shift in [false, true] {
                for (reading, book) in [(false, false), (true, false)] {
                    let table = with_shift(bindings_in_force(reading, book), shift);
                    assert!(!table.iter().any(|(k, _)| *k == key), "{key:?} is bound at ({reading}, {book}, {shift})");
                }
            }
        }
        let ways: Vec<f64> = slide_bindings().iter().map(|(_, w)| *w).collect();
        assert!(ways.contains(&1.0) && ways.contains(&-1.0), "both ways need a key");
    }

    #[test]
    fn h_is_the_editor() {
        assert_eq!(letter_for(&Action::ToggleForm), Some('H'));
    }

    /// Shift turns the wheel into the slide in the editor and nowhere else; `=` and `-` zoom the
    /// editor through the same `Zoom` the wheel sends, which the dispatcher routes by view.
    #[test]
    fn the_wheel_zooms_everywhere_and_slides_with_shift_in_the_editor() {
        use crate::ui::ViewMode;
        assert_eq!(wheel_action(ViewMode::Form, true, 2.0), Action::SlideForm(2.0 * crate::form_view::SLIDE_PER_NOTCH));
        assert_eq!(wheel_action(ViewMode::Form, false, 2.0), Action::Zoom(2.0));
        assert_eq!(wheel_action(ViewMode::World, true, 2.0), Action::Zoom(2.0));
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

    /// **Bevy UI is a surface too.** A right-press on one of the editor's controls must not take
    /// the cursor, the way one on an egui panel does not.
    #[test]
    fn a_press_on_a_bevy_control_does_not_take_the_view() {
        let (mut app, window) = harness();
        let button = app.world_mut().spawn(Interaction::Hovered).id();
        press(&mut app);
        app.update();
        assert_eq!(cursor(&app, window).grab_mode, CursorGrabMode::None, "the control's press turned the view");

        release(&mut app);
        app.update();
        app.world_mut().entity_mut(button).insert(Interaction::None);
        press(&mut app);
        app.update();
        assert_eq!(cursor(&app, window).grab_mode, CursorGrabMode::Locked, "off the control it is the view's");
    }

    /// In the editor the look button orbits the ship, locked exactly as it is over the sky.
    #[test]
    fn in_the_editor_the_look_orbits() {
        let (mut app, window) = harness();
        app.world_mut().resource_mut::<crate::app::Ui>().view = crate::ui::ViewMode::Form;
        press(&mut app);
        app.world_mut().resource_mut::<AccumulatedMouseMotion>().delta = Vec2::new(10.0, 0.0);
        app.update();
        assert_eq!(cursor(&app, window).grab_mode, CursorGrabMode::Locked);
        let sent: Vec<Action> =
            app.world_mut().resource_mut::<Messages<Requested>>().drain().map(|r| r.0).collect();
        assert!(matches!(sent.as_slice(), [Action::OrbitForm { .. }]), "{sent:?}");
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
        // An open book, not merely the device: the shelf has no pages, so there the arrows
        // still belong to the view.
        let mut state = app.world_mut().resource_mut::<crate::app::Ui>();
        state.open(Panel::Reader);
        state.reading.book = Some("something".to_owned());
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
