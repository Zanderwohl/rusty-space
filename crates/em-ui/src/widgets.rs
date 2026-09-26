//! Bevy-native menu widgets: a bordered panel of labels and buttons.
//!
//! Bevy UI rather than egui because these sit in front of a rendered background and have to
//! composite with it. The screens that are dense with text stay in egui.

use bevy::prelude::*;

use crate::theme::MenuTheme;

/// A button that reacts to hover. The colors ride on the component so the hover system needs
/// no theme of its own, and two screens in different palettes can be on screen at once.
#[derive(Component)]
pub struct MenuButton {
    rest: Color,
    hover: Color,
}

/// Spawns the widgets of one menu screen in a theme.
///
/// A builder rather than free functions taking a palette: a screen is a dozen calls and
/// threading `&mut Commands` and the theme through every one of them buries the text.
pub struct MenuUi<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    theme: MenuTheme,
    panel_width: f32,
    /// The face every label, message and button is set in. `None` is Bevy's own font.
    font: Option<Handle<Font>>,
    /// The face the title is set in, for a product whose name is a wordmark. Falls back to
    /// [`MenuUi::font`], because a screen whose heading is not the product's name — a modal,
    /// say — wants the interface face rather than a display one.
    title_font: Option<Handle<Font>>,
    title_size: f32,
}

impl<'a, 'w, 's> MenuUi<'a, 'w, 's> {
    pub const DEFAULT_PANEL_WIDTH: f32 = 400.0;
    pub const DEFAULT_TITLE_SIZE: f32 = 28.0;

    pub fn new(commands: &'a mut Commands<'w, 's>, theme: MenuTheme) -> Self {
        Self {
            commands,
            theme,
            panel_width: Self::DEFAULT_PANEL_WIDTH,
            font: None,
            title_font: None,
            title_size: Self::DEFAULT_TITLE_SIZE,
        }
    }

    /// Adds to or replaces a widget's components, for a caller laying it out its own way.
    pub fn insert(&mut self, entity: Entity, bundle: impl Bundle) {
        self.commands.entity(entity).insert(bundle);
    }

    pub fn panel_width(mut self, width: f32) -> Self {
        self.panel_width = width;
        self
    }

    /// Sets the whole screen in a face of the caller's own — the interface face, in a product
    /// that has one. Bevy UI has no font database to name a family in, so it is a handle, and
    /// the caller is the one holding it.
    pub fn font(mut self, font: Handle<Font>) -> Self {
        self.font = Some(font);
        self
    }

    /// Sets the title in a face of the caller's own, at a size that face wants. A display face
    /// is drawn at its own size or not at all — most of them are unreadable at the size a
    /// heading in the interface font is set at.
    pub fn title_font(mut self, font: Handle<Font>, size: f32) -> Self {
        self.title_font = Some(font);
        self.title_size = size;
        self
    }

    /// A full-screen node that centers whatever is put in it. The screen's own marker goes on
    /// it, so despawning that one entity takes the screen with it.
    pub fn screen(&mut self, marker: impl Bundle) -> Entity {
        self.commands
            .spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    position_type: PositionType::Absolute,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                marker,
            ))
            .id()
    }

    /// A screen with the backdrop drawn behind it, for something in front of what is already
    /// there.
    ///
    /// The same layout as [`MenuUi::screen`] plus a wash, so a modal is a screen that does not
    /// pretend the thing behind it has gone.
    ///
    /// Explicitly above everything. Bevy UI orders by spawn, and two systems spawning into one
    /// frame have no order between them — which draws an overlay *behind* what it is over,
    /// interleaved with it.
    pub fn overlay(&mut self, marker: impl Bundle) -> Entity {
        let screen = self.screen(marker);
        self.commands.entity(screen).insert((
            BackgroundColor(self.theme.overlay_backdrop),
            GlobalZIndex(OVERLAY_Z),
        ));
        screen
    }

    /// The bordered column every screen is built inside.
    pub fn panel(&mut self, parent: Entity) -> Entity {
        let panel = self
            .commands
            .spawn((
                Node {
                    width: Val::Px(self.panel_width),
                    min_height: Val::Px(200.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(30.0)),
                    row_gap: Val::Px(15.0),
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(self.theme.panel_bg),
                BorderColor::all(self.theme.border),
            ))
            .id();
        self.commands.entity(parent).add_child(panel);
        panel
    }

    pub fn title(&mut self, panel: Entity, text: &str) -> Entity {
        let font = self.title_font.clone().or_else(|| self.font.clone());
        let size = match self.title_font {
            Some(_) => self.title_size,
            // A display size is the display face's; the interface face at it would just be
            // a large label.
            None => Self::DEFAULT_TITLE_SIZE,
        };
        self.faced(panel, text, size, self.theme.text, font)
    }

    pub fn message(&mut self, panel: Entity, text: &str) -> Entity {
        self.label(panel, text, 18.0, self.theme.text_dim)
    }

    pub fn label(&mut self, panel: Entity, text: &str, font_size: f32, color: Color) -> Entity {
        let font = self.font.clone();
        self.faced(panel, text, font_size, color, font)
    }

    fn faced(
        &mut self,
        panel: Entity,
        text: &str,
        font_size: f32,
        color: Color,
        font: Option<Handle<Font>>,
    ) -> Entity {
        let label = self
            .commands
            .spawn((
                Text::new(text),
                TextFont {
                    // `FontSource::Handle` is the default variant, so `None` is still
                    // Bevy's own font rather than nothing.
                    font: font.map(FontSource::Handle).unwrap_or_default(),
                    font_size: FontSize::Px(font_size),
                    ..default()
                },
                TextColor(color),
                Node { margin: UiRect::bottom(Val::Px(10.0)), ..default() },
            ))
            .id();
        self.commands.entity(panel).add_child(label);
        label
    }

    /// A button carrying `action`, which the screen's handler reads back out of its
    /// `Interaction` query.
    pub fn button<A: Component>(&mut self, panel: Entity, text: &str, action: A) -> Entity {
        let node = Node {
            width: Val::Px(250.0),
            height: Val::Px(45.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(2.0)),
            ..default()
        };
        self.button_in(panel, text, action, node, 18.0)
    }

    fn button_in<A: Component>(
        &mut self,
        panel: Entity,
        text: &str,
        action: A,
        node: Node,
        font_size: f32,
    ) -> Entity {
        let theme = self.theme;
        let font = self.font.clone();
        let btn = self
            .commands
            .spawn((
                Button,
                node,
                BackgroundColor(theme.button_bg),
                BorderColor::all(theme.border),
                MenuButton { rest: theme.button_bg, hover: theme.button_hover },
                action,
            ))
            .with_children(|parent| {
                parent.spawn((
                    Text::new(text),
                    TextFont {
                        font: font.map(FontSource::Handle).unwrap_or_default(),
                        font_size: FontSize::Px(font_size),
                        ..default()
                    },
                    TextColor(theme.text),
                ));
            })
            .id();
        self.commands.entity(panel).add_child(btn);
        btn
    }
}

/// Which edge of the window a [`MenuUi::docked`] strip sits against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl<'a, 'w, 's> MenuUi<'a, 'w, 's> {
    /// Chrome over a view against one edge: full width at the top or bottom, a column at a side.
    /// It takes no pointer; only what is put in it does.
    pub fn docked(&mut self, marker: impl Bundle, edge: Edge, inset: f32) -> Entity {
        let mut node = Node {
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            ..default()
        };
        match edge {
            Edge::Top => node.top = Val::Px(inset),
            Edge::Bottom => node.bottom = Val::Px(inset),
            Edge::Left => node.left = Val::Px(inset),
            Edge::Right => node.right = Val::Px(inset),
        }
        match edge {
            Edge::Top | Edge::Bottom => node.width = Val::Percent(100.0),
            Edge::Left | Edge::Right => node.align_items = AlignItems::Stretch,
        }
        self.commands.spawn((node, marker)).id()
    }

    /// A bordered box, the compact sibling of [`MenuUi::panel`]: a [`MenuUi::row`] or two of
    /// labels and buttons, read at a glance rather than worked down.
    pub fn strip(&mut self, parent: Entity) -> Entity {
        let strip = self
            .commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(self.theme.panel_bg),
                BorderColor::all(self.theme.border),
            ))
            .id();
        self.commands.entity(parent).add_child(strip);
        strip
    }

    /// A line of widgets side by side, with no frame of its own.
    pub fn row(&mut self, parent: Entity) -> Entity {
        let row = self
            .commands
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(12.0),
                ..default()
            })
            .id();
        self.commands.entity(parent).add_child(row);
        row
    }

    /// A label for a [`MenuUi::row`]: no margin, since a row spaces its own children.
    pub fn inline(&mut self, parent: Entity, text: &str, font_size: f32, color: Color) -> Entity {
        let font = self.font.clone();
        let label = self.faced(parent, text, font_size, color, font);
        self.commands.entity(label).insert(Node::default());
        label
    }

    /// A [`MenuUi::button`] sized to its words, for a strip.
    pub fn small_button<A: Component>(&mut self, parent: Entity, text: &str, action: A) -> Entity {
        let node = Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(3.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        };
        self.button_in(parent, text, action, node, 15.0)
    }
}

impl<'a, 'w, 's> MenuUi<'a, 'w, 's> {
    /// Chosen wears the hover color at rest. Say so in its words too: color is never the only
    /// signal.
    pub fn chosen_button<A: Component>(&mut self, parent: Entity, text: &str, chosen: bool, action: A) -> Entity {
        let button = self.small_button(parent, text, action);
        if chosen {
            let theme = self.theme;
            self.commands.entity(button).insert((
                BackgroundColor(theme.button_hover),
                MenuButton { rest: theme.button_hover, hover: theme.button_hover },
            ));
        }
        button
    }

    /// A [`MenuUi::small_button`] that cannot be pressed: dimmed, no hover, no action. Style says a
    /// disabled button also says why, so the caller puts the reason beside it.
    ///
    /// It keeps an [`Interaction`] so the pointer over it is still a control's, and a view reading
    /// the mouse stands down there as it does over the live button.
    pub fn disabled_button(&mut self, parent: Entity, text: &str) -> Entity {
        let theme = self.theme;
        let font = self.font.clone();
        let button = self
            .commands
            .spawn((
                Node {
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(3.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(theme.panel_bg),
                BorderColor::all(theme.text_dim),
                Interaction::default(),
            ))
            .with_children(|parent| {
                parent.spawn((
                    Text::new(text),
                    TextFont { font: font.map(FontSource::Handle).unwrap_or_default(), font_size: FontSize::Px(15.0), ..default() },
                    TextColor(theme.text_dim),
                ));
            })
            .id();
        self.commands.entity(parent).add_child(button);
        button
    }

    /// Left-aligned, indented by `depth`, the width of its column.
    pub fn tree_row<A: Component>(&mut self, parent: Entity, depth: usize, text: &str, chosen: bool, action: A) -> Entity {
        let row = self.chosen_button(parent, text, chosen, action);
        self.commands.entity(row).insert(Node {
            padding: UiRect { left: Val::Px(6.0 + TREE_INDENT * depth as f32), right: Val::Px(6.0), top: Val::Px(2.0), bottom: Val::Px(2.0) },
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(if chosen { 1.0 } else { 0.0 })),
            ..default()
        });
        row
    }

    /// `marker` is what the caller finds its [`Committed`](crate::field::Committed) by.
    pub fn field<A: Component>(&mut self, parent: Entity, label: &str, text: &str, marker: A) -> Entity {
        self.fields(parent, label, [(text.to_owned(), marker)])[0]
    }

    /// Several fields on one labeled line, as the components of a vector.
    pub fn fields<A: Component>(&mut self, parent: Entity, label: &str, cells: impl IntoIterator<Item = (String, A)>) -> Vec<Entity> {
        let theme = self.theme;
        let font = self.font.clone();
        let row = self
            .commands
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .id();
        self.commands.entity(parent).add_child(row);
        self.inline(row, label, FIELD_TEXT, theme.text_dim);
        let cells: Vec<(String, A)> = cells.into_iter().collect();
        let line = self.commands.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(4.0), ..default() }).id();
        self.commands.entity(row).add_child(line);
        let glyphs = (FIELD_GLYPHS / cells.len().max(1) as f32).max(FIELD_LEAST_GLYPHS);
        let mut spawned = Vec::new();
        for (text, marker) in cells {
            let mut editable = bevy::text::EditableText::new(&text);
            editable.visible_width = Some(glyphs);
            editable.max_characters = Some(24);
            let field = self
                .commands
                .spawn((
                    Node { padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)), border: UiRect::all(Val::Px(1.0)), ..default() },
                    editable,
                    bevy::text::EditableTextFilter::new(crate::field::numeric),
                    bevy::text::TextCursorStyle { color: theme.text, ..default() },
                    TextLayout::no_wrap(),
                    TextFont { font: font.clone().map(FontSource::Handle).unwrap_or_default(), font_size: FontSize::Px(FIELD_TEXT), ..default() },
                    TextColor(theme.text),
                    BackgroundColor(theme.field_bg),
                    BorderColor::all(theme.border),
                    // So the pointer over it is a control's, and the view behind stands down.
                    Interaction::default(),
                    crate::field::NumberField::default(),
                    marker,
                ))
                .id();
            self.commands.entity(line).add_child(field);
            spawned.push(field);
        }
        spawned
    }
}

const TREE_INDENT: f32 = 12.0;
const FIELD_TEXT: f32 = 13.0;
/// Shared among a line's fields: one takes `-1.2345e-6`, each of three a short number.
const FIELD_GLYPHS: f32 = 16.0;
const FIELD_LEAST_GLYPHS: f32 = 4.0;

/// How far above the ordinary screens an overlay sits. Room underneath for anything that wants
/// to be between.
pub const OVERLAY_Z: i32 = 100;

pub fn button_hover_system(
    mut buttons: Query<
        (&Interaction, &MenuButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (interaction, button, mut color) in &mut buttons {
        *color = match *interaction {
            Interaction::Hovered | Interaction::Pressed => button.hover.into(),
            Interaction::None => button.rest.into(),
        };
    }
}
