//! Whether the pointer belongs to a Bevy UI control rather than to the view behind it.
//!
//! A view that reads the mouse itself — a camera turned by a drag — has to stand down over a
//! control, or a press on a button also turns the scene behind it. egui answers this for its
//! own surfaces; this is the same question for Bevy UI.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// Whether any of these says the pointer is on its control.
pub fn engaged<'a>(interactions: impl IntoIterator<Item = &'a Interaction>) -> bool {
    interactions.into_iter().any(|i| *i != Interaction::None)
}

/// Every control's [`Interaction`], as one answer.
#[derive(SystemParam)]
pub struct Controls<'w, 's> {
    interactions: Query<'w, 's, &'static Interaction>,
}

impl Controls<'_, '_> {
    /// Hovered or pressed. Hovered counts, so the answer is already right on the frame of the
    /// press, before the focus system has seen it.
    pub fn under_pointer(&self) -> bool {
        engaged(self.interactions.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hovered_or_pressed_control_holds_the_pointer() {
        assert!(!engaged(&[Interaction::None, Interaction::None]));
        assert!(engaged(&[Interaction::None, Interaction::Hovered]));
        assert!(engaged(&[Interaction::Pressed]));
        assert!(!engaged(&[]), "no controls, nothing to hold it");
    }

    #[derive(Resource, Default)]
    struct Seen(bool);

    fn probe(controls: Controls, mut seen: ResMut<Seen>) {
        seen.0 = controls.under_pointer();
    }

    #[test]
    fn the_parameter_reads_every_control_in_the_world() {
        let mut app = App::new();
        app.init_resource::<Seen>().add_systems(Update, probe);
        app.world_mut().spawn(Interaction::None);
        let button = app.world_mut().spawn(Interaction::None).id();
        app.update();
        assert!(!app.world().resource::<Seen>().0);
        *app.world_mut().entity_mut(button).get_mut::<Interaction>().unwrap() = Interaction::Pressed;
        app.update();
        assert!(app.world().resource::<Seen>().0);
    }
}
