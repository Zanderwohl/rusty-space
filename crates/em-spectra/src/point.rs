//! How a distant star is drawn: brightness in stops against a reference, a size and glare that
//! grow with it, and a core with a power-law halo. The client's starfield shader
//! (`lc-client/assets/shaders/starfield.wgsl`) evaluates the same laws from [`DISTANT`] and
//! [`POINT_STOPS`], so a star in a Galaxy Forge picture looks as it does in the game's sky. The
//! corona and the ship's motion are left out.

/// Stops of brightness a star field spreads over, beyond the tone map's own window.
pub const POINT_STOPS: f32 = 14.0;

/// Rec. 709 luminance weights, as the tone map uses.
pub const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// The laws' constants. The client's `PointStyle` carries these and its corona's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Drawn radius in pixels, faintest to brightest.
    pub min_px: f32,
    pub max_px: f32,
    /// Extra radius per stop above the window, in units of `min_px`.
    pub glow_radius_gain: f32,
    /// HDR value per stop above the window.
    pub overflow_gain: f32,
    /// Output gain for a source inside the window.
    pub brightness: f32,
    /// How much of the output the glare carries, against the source itself.
    pub halo_gain: f32,
    /// Exponent of the glare's power-law falloff from the source.
    pub halo_falloff: f32,
}

/// The background sky: small, tight, readable as a field of thousands.
pub const DISTANT: Look = Look {
    min_px: 1.0,
    max_px: 3.0,
    glow_radius_gain: 0.12,
    overflow_gain: 0.25,
    brightness: 1.2,
    halo_gain: 0.25,
    halo_falloff: 1.9,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    /// Linear display color at the center, HDR: above 1 where the star overflows the window.
    pub color: [f32; 3],
    /// Px.
    pub radius_px: f32,
    /// Where the source ends and the glare begins, as a fraction of the radius.
    pub core: f32,
}

impl Look {
    /// A star whose display-space light is `linear` against `reference`, the luminance at the
    /// top of the window. `None` for a star with no light.
    pub fn sprite(&self, linear: [f32; 3], reference: f32) -> Option<Sprite> {
        let luminance: f32 = linear.iter().zip(LUMA).map(|(c, w)| c * w).sum();
        let peak = linear.iter().copied().fold(0.0f32, f32::max);
        if !(luminance > 0.0) || reference <= 0.0 || peak <= 0.0 {
            return None;
        }
        let above = (luminance / reference).log2();
        let level = (1.0 + above / POINT_STOPS).clamp(0.0, 1.0);
        let glow = above.clamp(0.0, 24.0);
        let gain = self.brightness * (level + glow * self.overflow_gain);
        let radius_px = self.min_px + (self.max_px - self.min_px) * level + self.glow_radius_gain * glow * self.min_px;
        Some(Sprite { color: linear.map(|c| c / peak * gain), radius_px, core: (self.min_px / radius_px).clamp(0.0, 1.0) })
    }

    /// The sprite's weight at `r`, the distance from its center as a fraction of its radius:
    /// a bright edge to the source and a power-law fall outside it.
    pub fn profile(&self, sprite: &Sprite, r: f32) -> f32 {
        if r > 1.0 {
            return 0.0;
        }
        let core = 1.0 - smoothstep(sprite.core * 0.8, sprite.core, r);
        let halo = (sprite.core / r.max(sprite.core)).powf(self.halo_falloff) * (1.0 - smoothstep(0.55, 1.0, r));
        core + halo * self.halo_gain
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a).max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_brighter_star_is_bigger_and_overflows() {
        let dim = DISTANT.sprite([1.0, 1.0, 1.0], 100.0).unwrap();
        let bright = DISTANT.sprite([1.0, 1.0, 1.0], 0.01).unwrap();
        assert!(bright.radius_px > dim.radius_px && bright.color[0] > 1.0 && dim.color[0] < 1.0, "{dim:?} {bright:?}");
        assert!(DISTANT.profile(&bright, 0.0) > DISTANT.profile(&bright, 0.5) && DISTANT.profile(&bright, 1.01) == 0.0);
    }
}
