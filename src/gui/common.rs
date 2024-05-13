use bevy::prelude::*;

#[derive(Clone, Copy, Default, Eq, PartialEq, Debug, Hash, States)]
pub(crate) enum AppState {
    #[default]
    Splash,
    Menu,
    Planetarium,
}

// One of the two settings that can be set through the menu. It will be a resource in the app
#[derive(Resource, Debug, Component, PartialEq, Eq, Clone, Copy)]
pub(crate) enum DisplayQuality {
    Low,
    Medium,
    High,
}

#[derive(Resource, Debug, Component, PartialEq, Eq, Clone, Copy)]
pub(crate) enum BackGlow {
    None,
    Subtle,
    VFD,
    DEFCON,
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

pub mod panel {
    use bevy::prelude::{AlignItems, default, FlexDirection, Style, Val};
    use bevy::ui::UiRect;

    pub(crate) fn vertical() -> Style {
        Style {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect {
                left: Val::Px(10.0),
                right: Val::Px(10.0),
                top: Val::Px(10.0),
                bottom: Val::Px(10.0),
            },
            ..default()
        }
    }
}

pub mod text {
    use bevy::asset::AssetServer;
    use bevy::prelude::TextStyle;
    use crate::gui::common::color::TEXT_COLOR;

    pub(crate) fn primary(mut asset_server: AssetServer) -> TextStyle {
        TextStyle {
            font_size: 40.0,
            color: TEXT_COLOR,
            font: asset_server.load("fonts/Jost.ttf")
        }
    }
}

pub mod color {
    use bevy::prelude::Color;
    pub const BACKGROUND: Color = Color::rgb(0.69, 0.58, 0.33);
    pub const FOREGROUND: Color = Color::rgb(0.8, 0.73, 0.58);
    pub const HIGHLIGHT: Color = Color::rgb(0.65, 0.65, 0.65);

    pub const TEXT_COLOR: Color = Color::rgb(0.1, 0.1, 0.1);
    pub const NORMAL_BUTTON: Color = Color::rgb(0.79, 0.65, 0.05);
    pub const HOVERED_BUTTON: Color = Color::rgb(0.76, 0.69, 0.36);
    pub const HOVERED_PRESSED_BUTTON: Color = Color::rgb(0.76, 0.69, 0.36);
    pub const PRESSED_BUTTON: Color = Color::rgb(0.98, 0.89, 0.48);
}
