use bevy::prelude::*;
use bevy_egui::egui;
use crate::util::format;

pub fn despawn_entities_with<T: Component>(to_despawn: Query<Entity, With<T>>, mut commands: Commands) {
    for entity in &to_despawn {
        commands.entity(entity).despawn();
    }
}

pub fn despawn_recursive_entities_with<T: Component>(
    mut commands: Commands,
    query: Query<Entity, With<T>>,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
}

pub fn stepper<S: AsRef<str>>(ui: &mut egui::Ui, label: S, mut value: &mut f64) {
    ui.horizontal(|ui| {
       ui.label(label.as_ref());
        if ui.button("<<").clicked() { if *value > 0.0 { *value /= 10.0; } else { *value *= 10.0; } }
        if ui.button("<").clicked() { *value = bump_decimal(*value, -1.0); }
        ui.add(egui::DragValue::new(value)
            .speed(0.01)
            .range(f64::MIN..=f64::MAX)
            .fixed_decimals(1)
            .custom_formatter(|n, range| format::sci_not(n))
            .custom_parser(|s| format::sci_not_parser(s))
        );
        if ui.button(">").clicked() { *value = bump_decimal(*value, 1.0); }
        if ui.button(">>").clicked() { if *value > 0.0 { *value *= 10.0; } else { *value /= 10.0; } }
    });
}

fn bump_decimal(x: f64, direction: f64) -> f64 {
    if x == 0.0 { return 0.0; }

    // Find order of magnitude (e.g. 3.4e5 → exp = 5)
    let exp = x.abs().log10().floor();
    // Find the multiplier that makes the number ~[1, 10)
    let scale = 10f64.powf(exp);
    let normalized = x / scale; // e.g. 3.4

    // Change the first digit after the decimal (i.e. ±0.1)
    let bumped = (normalized * 10.0 + direction).round() / 10.0;

    bumped * scale.copysign(x)
}