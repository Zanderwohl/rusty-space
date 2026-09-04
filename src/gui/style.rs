// ============================================================================
// VFD Color Palette - shared across flat UI elements
// ============================================================================

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
