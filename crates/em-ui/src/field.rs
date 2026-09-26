//! A text field for a number, over Bevy's own [`EditableText`], which does the editing.
//!
//! Enter or leaving the field commits it, as a [`Committed`] naming the field for the caller to
//! read its own marker off; Escape puts back what it showed.

use bevy::ecs::system::SystemParam;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{FocusLost, FocusedInput, InputFocus};
use bevy::prelude::*;
use bevy::text::EditableText;

/// What it last showed, so a commit that changed nothing is not one.
#[derive(Component, Default)]
pub struct NumberField {
    shown: String,
}

#[derive(Message, Clone, Debug, PartialEq)]
pub struct Committed {
    pub field: Entity,
    pub value: f64,
}

pub fn numeric(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')
}

pub fn parse(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

impl NumberField {
    /// Unless it is being typed in.
    pub fn show(&mut self, editable: &mut EditableText, focused: bool, text: &str) {
        if focused || (self.shown == text && editable.value().to_string() == text) {
            return;
        }
        self.shown = text.to_owned();
        editable.editor_mut().set_text(text);
    }
}

/// Whether a text field has the keyboard, so key bindings stand down.
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

fn committed(field: &NumberField, editable: &EditableText) -> Option<f64> {
    let text = editable.value().to_string();
    (text != field.shown).then(|| parse(&text)).flatten()
}

fn on_key(
    key: On<FocusedInput<KeyboardInput>>,
    mut fields: Query<(&mut NumberField, &mut EditableText)>,
    mut focus: ResMut<InputFocus>,
    mut out: MessageWriter<Committed>,
) {
    let field = key.focused_entity;
    let Ok((mut number, mut editable)) = fields.get_mut(field) else { return };
    if !key.input.state.is_pressed() {
        return;
    }
    match key.input.logical_key {
        Key::Enter => {
            if let Some(value) = committed(&number, &editable) {
                out.write(Committed { field, value });
                // So losing the focus next does not commit it a second time.
                number.shown = editable.value().to_string();
            }
            focus.clear();
        }
        Key::Escape => {
            let shown = number.shown.clone();
            editable.editor_mut().set_text(&shown);
            focus.clear();
        }
        _ => {}
    }
}

fn on_focus_lost(lost: On<FocusLost>, fields: Query<(&NumberField, &EditableText)>, mut out: MessageWriter<Committed>) {
    if let Ok((number, editable)) = fields.get(lost.entity)
        && let Some(value) = committed(number, editable)
    {
        out.write(Committed { field: lost.entity, value });
    }
}

/// Bevy moves the focus only to something focusable, and a rendered view is not.
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

    /// A field showing `shown`, focused, with `typed` in it.
    fn app_with_field(shown: &str, typed: &str) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            bevy::input_focus::InputFocusPlugin,
            bevy::input_focus::InputDispatchPlugin,
            FieldPlugin,
        ));
        app.world_mut().spawn((bevy::window::Window::default(), bevy::window::PrimaryWindow));
        let mut number = NumberField::default();
        let mut editable = EditableText::default();
        number.show(&mut editable, false, shown);
        editable.editor_mut().set_text(typed);
        let field = app.world_mut().spawn((number, editable, Interaction::None)).id();
        app.world_mut().resource_mut::<InputFocus>().set(field, bevy::input_focus::FocusCause::Pressed);
        app.update();
        (app, field)
    }

    fn press(app: &mut App, logical_key: Key) {
        let window = app.world_mut().query_filtered::<Entity, With<bevy::window::PrimaryWindow>>().single(app.world()).unwrap();
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::Enter,
            logical_key,
            state: bevy::input::ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
        app.update();
    }

    fn commits(app: &App) -> Vec<Committed> {
        let messages = app.world().resource::<Messages<Committed>>();
        messages.get_cursor().read(messages).cloned().collect()
    }

    /// Runs the observers, which is the only check that their queries do not conflict.
    #[test]
    fn enter_commits_the_number_typed() {
        let (mut app, field) = app_with_field("1", "2.5e6");
        press(&mut app, Key::Enter);
        assert_eq!(commits(&app), vec![Committed { field, value: 2.5e6 }]);
        assert_eq!(app.world().resource::<InputFocus>().get(), None);
        let (mut app, _) = app_with_field("1", "1");
        press(&mut app, Key::Enter);
        assert!(commits(&app).is_empty(), "nothing changed, so nothing is committed");
    }

    #[test]
    fn escape_puts_back_what_it_showed_and_commits_nothing() {
        let (mut app, field) = app_with_field("7", "9");
        press(&mut app, Key::Escape);
        assert!(commits(&app).is_empty());
        assert_eq!(app.world().get::<EditableText>(field).unwrap().value().to_string(), "7");
    }

    #[test]
    fn a_press_elsewhere_takes_the_keyboard_back_and_commits() {
        let (mut app, field) = app_with_field("1", "3");
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        app.update();
        assert_eq!(app.world().resource::<InputFocus>().get(), None);
        assert_eq!(commits(&app), vec![Committed { field, value: 3.0 }]);
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
