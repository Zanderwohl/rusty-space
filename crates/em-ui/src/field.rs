//! A text field for a number, over Bevy's own [`EditableText`].
//!
//! Bevy 0.19 edits text itself, with `bevy_ui_widgets` turning keys and presses into edits, so
//! this is only what a number needs on top: which characters it takes, when it is committed, and
//! showing a value changed elsewhere without trampling one being typed.
//!
//! **Enter or leaving the field commits it; Escape puts back what it showed.** A commit is a
//! [`Committed`] message naming the field, which the caller reads its own marker off, as a button
//! carries its action.

use bevy::ecs::system::SystemParam;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{FocusLost, FocusedInput, InputFocus};
use bevy::prelude::*;
use bevy::text::EditableText;

/// A field holding a number. What it last showed, so a commit that changed nothing is not one.
#[derive(Component, Default)]
pub struct NumberField {
    shown: String,
}

/// A number typed and committed.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct Committed {
    pub field: Entity,
    pub value: f64,
}

/// What a number field lets through: digits, a point, a sign and an exponent.
pub fn numeric(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')
}

pub fn parse(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

impl NumberField {
    /// Show `text`, unless the field is being typed in: a value moved by a drag elsewhere reaches
    /// every field but the one with the keyboard.
    pub fn show(&mut self, editable: &mut EditableText, focused: bool, text: &str) {
        if focused || (self.shown == text && editable.value().to_string() == text) {
            return;
        }
        self.shown = text.to_owned();
        editable.editor_mut().set_text(text);
    }
}

/// Whether the keyboard belongs to a text field, so key bindings stand down while one is typed in.
#[derive(SystemParam)]
pub struct Typing<'w, 's> {
    focus: Option<Res<'w, InputFocus>>,
    fields: Query<'w, 's, (), With<EditableText>>,
}

impl Typing<'_, '_> {
    pub fn active(&self) -> bool {
        self.focus.as_ref().and_then(|f| f.get()).is_some_and(|e| self.fields.contains(e))
    }
}

pub struct FieldPlugin;

impl Plugin for FieldPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Committed>()
            .add_observer(on_key)
            .add_observer(on_focus_lost)
            .add_systems(PreUpdate, blur_on_press_elsewhere.after(bevy::ui::UiSystems::Focus));
    }
}

fn commit(entity: Entity, fields: &Query<(&NumberField, &EditableText)>, out: &mut MessageWriter<Committed>) {
    let Ok((field, editable)) = fields.get(entity) else { return };
    let text = editable.value().to_string();
    if text == field.shown {
        return;
    }
    if let Some(value) = parse(&text) {
        out.write(Committed { field: entity, value });
    }
}

fn on_key(
    key: On<FocusedInput<KeyboardInput>>,
    fields: Query<(&NumberField, &EditableText)>,
    mut reverting: Query<(&mut NumberField, &mut EditableText)>,
    mut focus: ResMut<InputFocus>,
    mut out: MessageWriter<Committed>,
) {
    let entity = key.focused_entity;
    if !key.input.state.is_pressed() || !fields.contains(entity) {
        return;
    }
    match key.input.logical_key {
        Key::Enter => {
            commit(entity, &fields, &mut out);
            focus.clear();
        }
        Key::Escape => {
            if let Ok((field, mut editable)) = reverting.get_mut(entity) {
                let shown = field.shown.clone();
                editable.editor_mut().set_text(&shown);
            }
            focus.clear();
        }
        _ => {}
    }
}

fn on_focus_lost(lost: On<FocusLost>, fields: Query<(&NumberField, &EditableText)>, mut out: MessageWriter<Committed>) {
    commit(lost.entity, &fields, &mut out);
}

/// A press anywhere but the field takes the keyboard from it, which commits it. Bevy moves the
/// focus only to something focusable, and a rendered view is not.
fn blur_on_press_elsewhere(
    buttons: Res<ButtonInput<MouseButton>>,
    focus: Option<ResMut<InputFocus>>,
    fields: Query<&Interaction, With<NumberField>>,
) {
    let Some(mut focus) = focus else { return };
    if !buttons.get_just_pressed().any(|_| true) {
        return;
    }
    let Some(focused) = focus.get() else { return };
    if fields.get(focused).is_ok_and(|i| *i == Interaction::None) {
        focus.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_takes_the_characters_of_a_number_and_nothing_else() {
        for c in "0123456789.-+eE".chars() {
            assert!(numeric(c), "{c}");
        }
        for c in "xh /,_".chars() {
            assert!(!numeric(c), "{c}");
        }
        assert_eq!(parse(" 2.5e6 "), Some(2.5e6));
        assert_eq!(parse("-0.3"), Some(-0.3));
        assert_eq!(parse("1e999"), None, "not a number a form can hold");
        assert_eq!(parse("--"), None);
    }

    #[derive(Resource, Default)]
    struct Seen(bool);

    fn probe(typing: Typing, mut seen: ResMut<Seen>) {
        seen.0 = typing.active();
    }

    #[test]
    fn the_keyboard_is_a_fields_only_while_one_has_the_focus() {
        let mut app = App::new();
        app.init_resource::<Seen>().init_resource::<InputFocus>().add_systems(Update, probe);
        let field = app.world_mut().spawn(EditableText::default()).id();
        let other = app.world_mut().spawn(Node::default()).id();
        app.update();
        assert!(!app.world().resource::<Seen>().0);
        app.world_mut().resource_mut::<InputFocus>().set(other, bevy::input_focus::FocusCause::Pressed);
        app.update();
        assert!(!app.world().resource::<Seen>().0, "focus on something that is not a field");
        app.world_mut().resource_mut::<InputFocus>().set(field, bevy::input_focus::FocusCause::Pressed);
        app.update();
        assert!(app.world().resource::<Seen>().0);
    }
}
