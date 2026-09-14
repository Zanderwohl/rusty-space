//! Bevy-native menu widgets: a bordered panel of labels and buttons.
//!
//! Bevy UI rather than egui because these sit in front of a rendered background and have to
//! composite with it. The screens that are dense with text stay in egui.

use bevy::prelude::*;

use crate::theme::MenuTheme;

/// A button that reacts to hover. The colours ride on the component so the hover system needs
/// no theme of its own, and two screens in different palettes can be on screen at once.
#[derive(Component)]
pub struct MenuButton {
    rest: Color,
    hover: Color,
}

/// Spawns the widgets of one menu screen in a theme.
///
/// A builder rather than free functions taking a palette: a screen is a dozen calls and
/// threading `&mut Commands` and the theme through every one of them buries the text.
pub struct MenuUi<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    theme: MenuTheme,
    panel_width: f32,
}

impl<'a, 'w, 's> MenuUi<'a, 'w, 's> {
    pub const DEFAULT_PANEL_WIDTH: f32 = 400.0;

    pub fn new(commands: &'a mut Commands<'w, 's>, theme: MenuTheme) -> Self {
        Self { commands, theme, panel_width: Self::DEFAULT_PANEL_WIDTH }
    }

    pub fn panel_width(mut self, width: f32) -> Self {
        self.panel_width = width;
        self
    }

    /// A full-screen node that centres whatever is put in it. The screen's own marker goes on
    /// it, so despawning that one entity takes the screen with it.
    pub fn screen(&mut self, marker: impl Bundle) -> Entity {
        self.commands
            .spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    position_type: PositionType::Absolute,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                marker,
            ))
            .id()
    }

    /// The bordered column every screen is built inside.
    pub fn panel(&mut self, parent: Entity) -> Entity {
        let panel = self
            .commands
            .spawn((
                Node {
                    width: Val::Px(self.panel_width),
                    min_height: Val::Px(200.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(30.0)),
                    row_gap: Val::Px(15.0),
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(self.theme.panel_bg),
                BorderColor::all(self.theme.border),
            ))
            .id();
        self.commands.entity(parent).add_child(panel);
        panel
    }

    pub fn title(&mut self, panel: Entity, text: &str) -> Entity {
        self.label(panel, text, 28.0, self.theme.text)
    }

    pub fn message(&mut self, panel: Entity, text: &str) -> Entity {
        self.label(panel, text, 18.0, self.theme.text_dim)
    }

    pub fn label(&mut self, panel: Entity, text: &str, font_size: f32, color: Color) -> Entity {
        let label = self
            .commands
            .spawn((
                Text::new(text),
                TextFont { font_size, ..default() },
                TextColor(color),
                Node { margin: UiRect::bottom(Val::Px(10.0)), ..default() },
            ))
            .id();
        self.commands.entity(panel).add_child(label);
        label
    }

    /// A button carrying `action`, which the screen's handler reads back out of its
    /// `Interaction` query.
    pub fn button<A: Component>(&mut self, panel: Entity, text: &str, action: A) -> Entity {
        let theme = self.theme;
        let btn = self
            .commands
            .spawn((
                Button,
                Node {
                    width: Val::Px(250.0),
                    height: Val::Px(45.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(theme.button_bg),
                BorderColor::all(theme.border),
                MenuButton { rest: theme.button_bg, hover: theme.button_hover },
                action,
            ))
            .with_children(|parent| {
                parent.spawn((
                    Text::new(text),
                    TextFont { font_size: 18.0, ..default() },
                    TextColor(theme.text),
                ));
            })
            .id();
        self.commands.entity(panel).add_child(btn);
        btn
    }
}

pub fn button_hover_system(
    mut buttons: Query<
        (&Interaction, &MenuButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (interaction, button, mut color) in &mut buttons {
        *color = match *interaction {
            Interaction::Hovered | Interaction::Pressed => button.hover.into(),
            Interaction::None => button.rest.into(),
        };
    }
}
