use bevy::asset::AssetServer;
use bevy::prelude::{AlignItems, ButtonBundle, Commands, Component, default, NodeBundle, Res, Style, TextBundle, Val};
use bevy::hierarchy::BuildChildren;
use crate::gui::common;
use crate::gui::common::{text, Volume};
use crate::gui::common::color::NORMAL_BUTTON;
use crate::gui::menu::main::{MenuButtonAction, SelectedOption};

#[derive(Component)]
pub struct VolumeSetting;

pub fn sound_settings_menu_setup(mut commands: Commands, volume: Res<Volume>, asset_server: Res<AssetServer>) {
    let button_style = common::button_style();
    let button_text_style = text::primary(asset_server.clone());

    let base_screen = common::base_screen(&mut commands);
    commands.entity(base_screen)
        .insert(OnSoundSettingsMenuScreen)
        .with_children(|parent| {
            parent
                .spawn(NodeBundle {
                    style: common::panel::vertical(),
                    background_color: common::color::FOREGROUND.into(),
                    ..default()
                })
                .with_children(|parent| {
                    parent
                        .spawn(NodeBundle {
                            style: Style {
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            background_color: common::color::FOREGROUND.into(),
                            ..default()
                        })
                        .with_children(|parent| {
                            parent.spawn(TextBundle::from_section(
                                "VOLUME",
                                button_text_style.clone(),
                            ));
                            for volume_setting in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] {
                                let mut entity = parent.spawn((
                                    ButtonBundle {
                                        style: Style {
                                            width: Val::Px(30.0),
                                            height: Val::Px(65.0),
                                            ..button_style.clone()
                                        },
                                        background_color: NORMAL_BUTTON.into(),
                                        ..default()
                                    },
                                    Volume(volume_setting),
                                    VolumeSetting,
                                ));
                                if *volume == Volume(volume_setting) {
                                    entity.insert(SelectedOption);
                                }
                            }
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
                            parent.spawn(TextBundle::from_section("BACK", button_text_style));
                        });
                });
        });
}

// Tag component used to tag entities added on the sound settings menu screen
#[derive(Component)]
pub(crate) struct OnSoundSettingsMenuScreen;
