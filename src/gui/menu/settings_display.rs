use bevy::asset::AssetServer;
use bevy::prelude::{AlignItems, ButtonBundle, Commands, Component, default, NodeBundle, Res, Style, TextBundle, Val};
use bevy::hierarchy::BuildChildren;
use crate::gui::common;
use crate::gui::common::{BackGlow, DisplayQuality, text};
use crate::gui::common::color::NORMAL_BUTTON;
use crate::gui::menu::main::{MenuButtonAction, SelectedOption};

#[derive(Component)]
pub struct QualitySetting;

#[derive(Component)]
pub struct GlowSetting;

pub fn display_settings_menu_setup(mut commands: Commands,
                                   display_quality: Res<DisplayQuality>,
                                   back_glow: Res<BackGlow>,
                                   asset_server: Res<AssetServer>) {
    let button_style = common::button_style();
    let button_text_style = text::primary(asset_server.clone());

    let base_screen = common::base_screen(&mut commands);
    commands.entity(base_screen)
        .insert(OnDisplaySettingsMenuScreen)
        .with_children(|parent| {
            parent
                .spawn(NodeBundle {
                    style: common::panel::vertical(),
                    background_color: common::color::FOREGROUND.into(),
                    ..default()
                })
                .with_children(|parent| {
                    // Create a new `NodeBundle`, this time not setting its `flex_direction`. It will
                    // use the default value, `FlexDirection::Row`, from left to right.
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
                            // Display a label for the current setting
                            parent.spawn(TextBundle::from_section(
                                "DISPLAY QUALITY",
                                button_text_style.clone(),
                            ));
                            // Display a button for each possible value
                            for quality_setting in [
                                DisplayQuality::Low,
                                DisplayQuality::Medium,
                                DisplayQuality::High,
                            ] {
                                let mut entity = parent.spawn((
                                    ButtonBundle {
                                        style: Style {
                                            width: Val::Px(150.0),
                                            height: Val::Px(65.0),
                                            ..button_style.clone()
                                        },
                                        background_color: NORMAL_BUTTON.into(),
                                        ..default()
                                    },
                                    quality_setting,
                                    QualitySetting,
                                ));
                                entity.with_children(|parent| {
                                    parent.spawn(TextBundle::from_section(
                                        format!("{quality_setting:?}"),
                                        button_text_style.clone(),
                                    ));
                                });
                                if *display_quality == quality_setting {
                                    entity.insert(SelectedOption);
                                }
                            }
                        });
                    // Glow settings
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
                            // Display a label for the current setting
                            parent.spawn(TextBundle::from_section(
                                "GLOW",
                                button_text_style.clone(),
                            ));
                            // Display a button for each possible value
                            for glow_setting in [
                                BackGlow::None,
                                BackGlow::Subtle,
                                BackGlow::VFD,
                                BackGlow::DEFCON,
                            ] {
                                let mut entity = parent.spawn((
                                    ButtonBundle {
                                        style: Style {
                                            width: Val::Px(150.0),
                                            height: Val::Px(65.0),
                                            ..button_style.clone()
                                        },
                                        background_color: NORMAL_BUTTON.into(),
                                        ..default()
                                    },
                                    glow_setting,
                                    GlowSetting,
                                ));
                                entity.with_children(|parent| {
                                    parent.spawn(TextBundle::from_section(
                                        format!("{glow_setting:?}"),
                                        button_text_style.clone(),
                                    ));
                                });
                                if *back_glow == glow_setting {
                                    entity.insert(SelectedOption);
                                }
                            }
                        });

                    // Display the back button to return to the settings screen
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

// Tag component used to tag entities added on the display settings menu screen
#[derive(Component)]
pub(crate) struct OnDisplaySettingsMenuScreen;
