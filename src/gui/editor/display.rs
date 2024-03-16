use bevy::prelude::*;
use crate::gui::common;
use crate::gui::common::text_style;

use super::super::common::{despawn_screen, DisplayQuality, AppState, Volume, TEXT_COLOR};

// This plugin will contain the game. In this case, it's just be a screen that will
// display the current settings for 5 seconds before returning to the menu
pub fn editor_plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Editor), editor_setup)
        .add_systems(Update, editor.run_if(in_state(AppState::Editor)))
        .add_systems(OnExit(AppState::Editor), despawn_screen::<OnEditorScreen>);
}

// Tag component used to tag entities added on the game screen
#[derive(Component)]
struct OnEditorScreen;

#[derive(Resource, Deref, DerefMut)]
struct GameTimer(Timer);

fn editor_setup(
    mut commands: Commands,
    display_quality: Res<DisplayQuality>,
    volume: Res<Volume>,
) {
    let base_screen = common::base_screen(&mut commands);
    commands.entity(base_screen)
        .insert(OnEditorScreen)
        .with_children(|parent| {
            // First create a `NodeBundle` for centering what we want to display
            parent
                .spawn(NodeBundle {
                    style: Style {
                        // This will display its children in a column, from top to bottom
                        flex_direction: FlexDirection::Column,
                        // `align_items` will align children on the cross axis. Here the main axis is
                        // vertical (column), so the cross axis is horizontal. This will center the
                        // children
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    background_color: Color::BLACK.into(),
                    ..default()
                })
                .with_children(|parent| {
                    // Display two lines of text, the second one with the current settings
                    parent.spawn(
                        TextBundle::from_section(
                            "Will be back to the menu shortly...",
                            common::text_style(),
                        )
                            .with_style(Style {
                                margin: UiRect::all(Val::Px(50.0)),
                                ..default()
                            }),
                    );
                    parent.spawn(
                        TextBundle::from_sections([
                            TextSection::new(
                                format!("quality: {:?}", *display_quality),
                                TextStyle {
                                    font_size: 60.0,
                                    color: Color::BLUE,
                                    ..default()
                                },
                            ),
                            TextSection::new(
                                " - ",
                                text_style(),
                            ),
                            TextSection::new(
                                format!("volume: {:?}", *volume),
                                TextStyle {
                                    font_size: 60.0,
                                    color: Color::GREEN,
                                    ..default()
                                },
                            ),
                        ])
                            .with_style(Style {
                                margin: UiRect::all(Val::Px(50.0)),
                                ..default()
                            }),
                    );
                });
        });
    // Spawn a 5 seconds timer to trigger going back to the menu
    commands.insert_resource(GameTimer(Timer::from_seconds(5.0, TimerMode::Once)));
}

// Tick the timer, and change state when finished
fn editor(
    time: Res<Time>,
    mut game_state: ResMut<NextState<AppState>>,
    mut timer: ResMut<GameTimer>,
) {
    if timer.tick(time.delta()).finished() {
        game_state.set(AppState::Menu);
    }
}
