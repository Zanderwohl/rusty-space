use bevy::prelude::{AlignItems, BuildChildren, ButtonBundle, Color, Commands, Component, default, FlexDirection, NodeBundle, Res, Style, TextBundle, Val};
use crate::gui::common;
use crate::gui::common::{text, Volume};
use crate::gui::common::color::NORMAL_BUTTON;
use crate::gui::menu::main::{MenuButtonAction, SelectedOption};

#[derive(Component)]
pub(crate) struct OnUITestScreen;

pub fn ui_test_menu_setup(mut commands: Commands) {
    let button_style = common::button_style();
    let button_text_style = text::primary();

    let base_screen = common::base_screen(&mut commands);
    commands.entity(base_screen)
        .insert(OnUITestScreen)
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
                    parent
                        .spawn(NodeBundle {
                            style: Style {
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            background_color: Color::CRIMSON.into(),
                            ..default()
                        })
                        .with_children(|parent| {
                            parent.spawn(TextBundle::from_section(
                                "UI TEST",
                                button_text_style.clone(),
                            ));
                        });
                    parent
                        .spawn((
                            ButtonBundle {
                                style: button_style,
                                background_color: NORMAL_BUTTON.into(),
                                ..default()
                            },
                            MenuButtonAction::BackToSettings,
                        ))
                        .with_children(|parent| {
                            parent.spawn(TextBundle::from_section("Back", button_text_style));
                        });
                });
        });
}
