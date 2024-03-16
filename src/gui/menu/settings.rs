use bevy::prelude::{AlignItems, ButtonBundle, Color, Commands, Component, default, FlexDirection, NodeBundle, Style, TextBundle};
use bevy::hierarchy::BuildChildren;
use crate::gui::common;
use crate::gui::common::color::NORMAL_BUTTON;
use crate::gui::common::text;
use crate::gui::menu::main::{MenuButtonAction, OnSettingsMenuScreen};

pub fn settings_menu_setup(mut commands: Commands) {
    let button_style = common::button_style();
    let button_text_style = text::primary();

    let base_screen = common::base_screen(&mut commands);
    commands.entity(base_screen)
        .insert(OnSettingsMenuScreen)
        .with_children(|parent| {
            parent
                .spawn(NodeBundle {
                    style: Style {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    background_color: Color::CRIMSON.into(),
                    ..default()
                })
                .with_children(|parent| {
                    for (action, text) in [
                        (MenuButtonAction::SettingsDisplay, "Display"),
                        (MenuButtonAction::SettingsSound, "Sound"),
                        (MenuButtonAction::BackToMainMenu, "Back"),
                    ] {
                        parent
                            .spawn((
                                ButtonBundle {
                                    style: button_style.clone(),
                                    background_color: NORMAL_BUTTON.into(),
                                    ..default()
                                },
                                action,
                            ))
                            .with_children(|parent| {
                                parent.spawn(TextBundle::from_section(
                                    text,
                                    button_text_style.clone(),
                                ));
                            });
                    }
                });
        });
}
