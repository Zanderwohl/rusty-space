use bevy::asset::AssetServer;
use bevy::prelude::{AlignItems, BuildChildren, ButtonBundle, Commands, Component, default, NodeBundle, Res, Style, TextBundle};
use crate::gui::common;
use crate::gui::common::{text};
use crate::gui::common::color::NORMAL_BUTTON;
use crate::gui::menu::main::{MenuButtonAction};

#[derive(Component)]
pub(crate) struct OnUITestScreen;

pub fn ui_test_menu_setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let button_style = common::button_style();
    let button_text_style = text::primary(asset_server.clone());

    let base_screen = common::base_screen(&mut commands);
    commands.entity(base_screen)
        .insert(OnUITestScreen)
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
                            parent.spawn(TextBundle::from_section("BACK", button_text_style));
                        });
                });
        });
}
