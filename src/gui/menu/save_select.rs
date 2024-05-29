use std::ffi::OsStr;
use std::fs::{DirEntry, FileType};
use std::path::PathBuf;
use bevy::hierarchy::ChildBuilder;
use bevy::prelude::{AssetServer, BuildChildren, ButtonBundle, Commands, Component, default, info, NodeBundle, Res, Resource, Style, TextBundle, TextStyle, UiRect, Val};
use crate::gui::common;
use crate::gui::common::color::{BACKGROUND, HIGHLIGHT, NORMAL_BUTTON};
use crate::gui::common::text;
use crate::gui::menu::main::MenuButtonAction;

pub fn save_select_setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let button_style = common::button_style();
    let button_text_style = text::primary(asset_server.clone());

    let templates = get_saves("assets/templates");
    let saves = get_saves("saves");

    let base_screen = common::base_screen(&mut commands);
    commands.entity(base_screen)
        .insert(OnSaveSelectScreen)
        .with_children(|parent| {
            parent
                .spawn(NodeBundle {
                    style: common::panel::vertical(),
                    background_color: common::color::FOREGROUND.into(),
                    ..default()
                })
                .with_children(|parent| {
                    parent
                        .spawn((
                            ButtonBundle {
                                style: button_style.clone(),
                                background_color: NORMAL_BUTTON.into(),
                                ..default()
                            },
                            MenuButtonAction::BackToMainMenu,
                        ))
                        .with_children(|parent| {
                            parent.spawn(TextBundle::from_section(
                                "BACK",
                                button_text_style.clone(),
                            ));
                        });
                    parent
                        .spawn(NodeBundle {
                            style: common::panel::horizontal(),
                            background_color: common::color::FOREGROUND.into(),
                            ..default()
                        }).with_children(|parent| {
                        parent
                            .spawn(NodeBundle {
                                background_color: BACKGROUND.into(),
                                style: common::panel::vertical_with_margin(),
                                ..default()
                            }).with_children(|parent| {
                            parent.spawn(
                                TextBundle::from_section(
                                    "New",
                                    button_text_style.clone(),
                                )
                                    .with_style(Style {
                                        margin: UiRect::all(Val::Px(50.0)),
                                        ..default()
                                    }),
                            );
                            for template in templates.iter() {
                                save_file_button(parent, &template, button_style.clone(), button_text_style.clone());
                            }
                        });
                        parent
                            .spawn(NodeBundle {
                                background_color: HIGHLIGHT.into(),
                                style: common::panel::vertical_with_margin(),
                                ..default()
                            }).with_children(|parent| {
                            parent.spawn(
                                TextBundle::from_section(
                                    "Load",
                                    button_text_style.clone(),
                                )
                                    .with_style(Style {
                                        margin: UiRect::all(Val::Px(50.0)),
                                        ..default()
                                    }),
                            );
                            for save in saves.iter() {
                                save_file_button(parent, &save, button_style.clone(), button_text_style.clone());
                            }
                        });
                    });
                });
        });
}

fn save_file_button(parent_bundle: &mut ChildBuilder, template: &&SaveEntry, button_style: Style, text_style: TextStyle) {
    let name = &template.name.to_uppercase();
    parent_bundle.spawn((
        ButtonBundle {
            style: button_style.clone(),
            background_color: NORMAL_BUTTON.into(),
            ..default()
        },
        MenuButtonAction::LoadSave(template.clone().clone()),
    ))
        .with_children(|parent| {
            parent.spawn(TextBundle::from_section(
                name,
                text_style.clone(),
            ));
        });
}

#[derive(Debug, Resource, Clone)]
pub struct SaveEntry {
    pub path: PathBuf,
    pub name: String,
}


fn get_saves(dir: &str) -> Vec<SaveEntry> {
    use std::fs;

    let paths = fs::read_dir(dir).unwrap(); // Unwrap ok? App starts with touching this.

    let saves: Vec<_> = paths
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let path = entry.path();
            let path_str = path.to_string_lossy();
            path_str.ends_with("yml") || path_str.ends_with("yaml")
        }
        )
        .map(|dir| dir.path())
        .collect();
    let stems: Vec<String> = saves.iter().filter_map(|entry| {
        entry.file_stem()
            .and_then(|stem| stem.to_str())
            .map(|s| s.to_string())
    }).collect();

    let mut save_entries = Vec::new();
    for (idx, path) in saves.iter().enumerate() {
        let save = SaveEntry {
            path: path.clone(),
            name: stems[idx].clone(),
        };
        save_entries.push(save);
    }
    save_entries
}

/// https://stackoverflow.com/questions/72392835/check-if-a-file-is-of-a-given-type
fn is_filetype(entry: DirEntry, _type: &str) -> bool {
    entry
        .file_name()
        .to_str()
        .map_or(false, |s| s.to_lowercase().ends_with(_type))
}

#[derive(Component)]
pub(crate) struct OnSaveSelectScreen;
