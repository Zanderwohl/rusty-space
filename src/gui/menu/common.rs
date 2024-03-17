use bevy::prelude::{BackgroundColor, Button, Changed, Interaction, Query, With};
use crate::gui::common::color::{HOVERED_BUTTON, HOVERED_PRESSED_BUTTON, NORMAL_BUTTON, PRESSED_BUTTON};
use crate::gui::menu::main::SelectedOption;

// This system handles changing all buttons color based on mouse interaction
pub(crate) fn button_system(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor, Option<&SelectedOption>),
        (Changed<Interaction>, With<Button>),
    >,
) {
    for (interaction, mut color, selected) in &mut interaction_query {
        *color = match (*interaction, selected) {
            (Interaction::Pressed, _) | (Interaction::None, Some(_)) => PRESSED_BUTTON.into(),
            (Interaction::Hovered, Some(_)) => HOVERED_PRESSED_BUTTON.into(),
            (Interaction::Hovered, None) => HOVERED_BUTTON.into(),
            (Interaction::None, None) => NORMAL_BUTTON.into(),
        }
    }
}
