use bevy::ecs::system::EntityCommands;
use bevy::prelude::*;

#[derive(Clone, Copy, Default, Eq, PartialEq, Debug, Hash, States)]
pub(crate) enum AppState {
    #[default]
    Splash,
    Menu,
    Editor,
}

// One of the two settings that can be set through the menu. It will be a resource in the app
#[derive(Resource, Debug, Component, PartialEq, Eq, Clone, Copy)]
pub(crate) enum DisplayQuality {
    Low,
    Medium,
    High,
}

// One of the two settings that can be set through the menu. It will be a resource in the app
#[derive(Resource, Debug, Component, PartialEq, Eq, Clone, Copy)]
pub(crate) struct Volume(pub u32);


// Generic system that takes a component as a parameter, and will despawn all entities with that component
pub(crate) fn despawn_screen<T: Component>(to_despawn: Query<Entity, With<T>>, mut commands: Commands) {
    for entity in &to_despawn {
        commands.entity(entity).despawn_recursive();
    }
}

pub const TEXT_COLOR: Color = Color::rgb(0.9, 0.9, 0.9);

pub(crate) fn base_screen(mut commands: &mut Commands) -> Entity {
    commands.spawn((
        NodeBundle {
            style: Style {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                // center children
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            ..default()
        },
    )).id()
}

pub(crate) fn button_style() -> Style {
    Style {
        width: Val::Px(200.0),
        height: Val::Px(65.0),
        margin: UiRect::all(Val::Px(20.0)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    }
}

pub(crate) fn text_style() -> TextStyle {
    TextStyle {
        font_size: 40.0,
        color: TEXT_COLOR,
        ..default()
    }
}
