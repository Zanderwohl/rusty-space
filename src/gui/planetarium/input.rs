use bevy::prelude::*;
use bevy_egui::EguiContexts;
use crate::body::motive::calculate_body_positions::PhysicsGraph;
use crate::camera::{GoTo, GoToSource, PlanetariumCamera};
use crate::gui::planetarium::FocusedBodyState;

pub fn handle_go_to_shortcut(
    keys: Res<ButtonInput<KeyCode>>,
    mut contexts: EguiContexts,
    focused_body_state: Res<FocusedBodyState>,
    physics_graph: Res<PhysicsGraph>,
    mut go_to: MessageWriter<GoTo>,
) {
    let go_to_source = if keys.just_pressed(KeyCode::KeyG) {
        Some(GoToSource::KeyboardG)
    } else if keys.just_pressed(KeyCode::KeyF) {
        Some(GoToSource::KeyboardF)
    } else {
        None
    };

    let Some(source) = go_to_source else {
        return;
    };

    if egui_wants_keyboard(&mut contexts) {
        return;
    }

    let Some(selected_id) = focused_body_state.current_body_id.as_deref() else {
        return;
    };

    // O(1) lookup via PhysicsGraph instead of O(n) iterator scan
    if let Some(&entity) = physics_graph.id_to_entity.get(selected_id) {
        info!("cam.input.goto source={:?} entity={:?}", source, entity);
        go_to.write(GoTo {
            entity,
            frame: None,
            source,
        });
    }
}

pub fn handle_revolve_frame_shortcut(
    keys: Res<ButtonInput<KeyCode>>,
    mut contexts: EguiContexts,
    camera: Query<&PlanetariumCamera>,
    mut go_to: MessageWriter<GoTo>,
) {
    if !keys.just_pressed(KeyCode::KeyV) || egui_wants_keyboard(&mut contexts) {
        return;
    }

    let Ok(camera) = camera.single() else {
        return;
    };

    if let Some((entity, frame)) = camera.action.revolve_target() {
        let next = frame.next();
        info!(
            "cam.input.frame_cycle source=KeyboardV entity={:?} from={:?} to={:?} action=RevolveAround",
            entity,
            frame,
            next
        );
        go_to.write(GoTo {
            entity,
            frame: Some(next),
            source: GoToSource::KeyboardV,
        });
        return;
    }

    if let Some((entity, frame)) = camera.action.goto_target() {
        let next = frame.next();
        info!(
            "cam.input.frame_cycle source=KeyboardV entity={:?} from={:?} to={:?} action=Goto",
            entity,
            frame,
            next
        );
        go_to.write(GoTo {
            entity,
            frame: Some(next),
            source: GoToSource::KeyboardV,
        });
    }
}

fn egui_wants_keyboard(contexts: &mut EguiContexts) -> bool {
    contexts
        .ctx_mut()
        .map_or(false, |ctx| ctx.wants_keyboard_input())
}
