use bevy::asset::AssetServer;
use bevy::prelude::{AlignItems, ButtonBundle, Color, Commands, Component, default, FlexDirection, NodeBundle, Res, Style, TextBundle};
use bevy::hierarchy::BuildChildren;
use crate::gui::common;
use crate::gui::common::color::NORMAL_BUTTON;
use crate::gui::common::text;
use crate::gui::menu::main::MenuButtonAction;

pub fn settings_menu_setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let button_style = common::button_style();
    let button_text_style = text::primary(asset_server.clone());

    let base_screen = common::base_screen(&mut commands);
    commands.entity(base_screen)
        .insert(OnSettingsMenuScreen)
        .with_children(|parent| {
            parent
                .spawn(NodeBundle {
                    style: common::panel::vertical(),
                    background_color: common::color::FOREGROUND.into(),
                    ..default()
                })
                .with_children(|parent| {
                    for (action, text) in [
                        (MenuButtonAction::SettingsDisplay, "DISPLAY"),
                        (MenuButtonAction::SettingsSound, "AUDIO"),
                        (MenuButtonAction::SettingsUITest, "UI TEST"),
                        (MenuButtonAction::BackToMainMenu, "BACK"),
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

// Tag component used to tag entities added on the settings menu screen
#[derive(Component)]
pub(crate) struct OnSettingsMenuScreen;
