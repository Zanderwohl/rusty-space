// crate::tonemap's surface curve, for shaders that evaluate it per fragment. Binding-free.
//
// A surface's linear display light goes to a window `stops` wide whose top is `reference`, on a
// logarithmic scale, with the hue carried separately as the light over its brightest channel.

/// Rec. 709, matching crate::tonemap.
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

/// Where `linear` sits in the window, `0..1`. Zero for no light or no window.
fn tone_value(linear: vec3<f32>, reference: f32, stops: f32) -> f32 {
    let luminance = dot(linear, LUMA);
    if (luminance > 0.0 && reference > 0.0 && stops > 0.0) {
        return clamp(log2(luminance / reference) / stops + 1.0, 0.0, 1.0);
    }
    return 0.0;
}

/// `linear` over its brightest channel; white for no light.
fn tone_chroma(linear: vec3<f32>) -> vec3<f32> {
    let peak = max(linear.r, max(linear.g, linear.b));
    return select(vec3<f32>(1.0), linear / peak, peak > 0.0);
}
