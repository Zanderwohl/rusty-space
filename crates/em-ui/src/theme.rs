//! The colors a menu is drawn in.

use bevy::color::Color;

/// The vacuum-fluorescent palette: green on near-black, as Exotic Matters draws everything
/// flat.
pub mod vfd {
    use bevy::color::Color;

    pub const PANEL_BG: Color = Color::srgba(0.04, 0.10, 0.06, 0.92);
    pub const BUTTON_BG: Color = Color::srgb(0.10, 0.23, 0.15);
    pub const BUTTON_HOVER: Color = Color::srgb(0.16, 0.35, 0.21);
    pub const BUTTON_BORDER: Color = Color::srgb(0.23, 0.73, 0.40);
    pub const TEXT: Color = Color::srgb(0.35, 0.93, 0.69);
    pub const TEXT_DIM: Color = Color::srgb(0.20, 0.55, 0.40);
    pub const OVERLAY_BACKDROP: Color = Color::srgba(0.0, 0.0, 0.0, 0.70);
}

/// Every color a menu screen needs, carried by value so two products - or two screens of one
/// product - can differ without a global.
#[derive(Clone, Copy, Debug)]
pub struct MenuTheme {
    pub panel_bg: Color,
    pub button_bg: Color,
    pub button_hover: Color,
    pub border: Color,
    pub text: Color,
    pub text_dim: Color,
    /// The wash behind a modal. See [`MenuUi::overlay`](crate::widgets::MenuUi::overlay).
    pub overlay_backdrop: Color,
}

impl MenuTheme {
    pub const VFD: Self = Self {
        panel_bg: vfd::PANEL_BG,
        button_bg: vfd::BUTTON_BG,
        button_hover: vfd::BUTTON_HOVER,
        border: vfd::BUTTON_BORDER,
        text: vfd::TEXT,
        text_dim: vfd::TEXT_DIM,
        overlay_backdrop: vfd::OVERLAY_BACKDROP,
    };
}

impl Default for MenuTheme {
    fn default() -> Self {
        Self::VFD
    }
}
