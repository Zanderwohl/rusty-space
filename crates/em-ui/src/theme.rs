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

    /// The second phosphor: a craft, against everything else.
    ///
    /// **The same perceptual lightness as [`TEXT`]**, so a ship and a body are equal weight on
    /// a screen and only their hue differs — they are two kinds of thing, not one more
    /// important than the other. Against that, as chromatic as sRGB reaches at that lightness,
    /// which is what makes it amber rather than cream: Oklab `L = 0.852`, `C = 0.164`, and the
    /// gamut boundary is at `C = 0.164`.
    pub const AMBER: Color = Color::srgb(1.0, 0.77, 0.18);
    pub const OVERLAY_BACKDROP: Color = Color::srgba(0.0, 0.0, 0.0, 0.70);
}

#[cfg(test)]
mod tests {
    use super::vfd;
    use bevy::color::Oklaba;

    /// **A ship and a body are the same weight on screen.** Only the hue says which is which,
    /// so the two have to sit at one perceptual lightness — a brighter amber would read as a
    /// warning and a darker one as a thing already dealt with.
    #[test]
    fn the_amber_is_the_green_lightness() {
        let green = Oklaba::from(vfd::TEXT).lightness;
        let amber = Oklaba::from(vfd::AMBER).lightness;
        assert!((amber - green).abs() < 0.01, "green {green}, amber {amber}");
        // And it is a warm hue, not a second green: Oklab `b` is what says so.
        assert!(Oklaba::from(vfd::AMBER).b > 0.1);
    }
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
