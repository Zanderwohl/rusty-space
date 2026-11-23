// A hull's depth seen from its star, into its own shadow map. See `lc_client::hull_shadow`.

struct Caster {
    light_from_ship: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> caster: Caster;

@vertex
fn vertex(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return caster.light_from_ship * vec4<f32>(position, 1.0);
}
