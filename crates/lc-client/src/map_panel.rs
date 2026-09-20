//! The two surfaces the map is shown on, and the input they take.
//!
//! One rendered image inside an egui surface, which is a third case beside the two
//! `lightcone/docs/18-ui-style.md` names. What makes it behave is allocating the image with an
//! explicit [`egui::Sense`] rather than calling `ui.image`: an interactive allocation is what
//! makes egui *want* the pointer, and `crate::input`'s wheel and cursor grab both already stand
//! down when it does. No new coordination, no flag, no ordering constraint.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::map::Map;
use crate::map_source::Source;
use crate::panels::ask;
use crate::ui::Panel;

/// Radians of turn per point of drag. The same feel as the sky's own look.
const TURN_PER_POINT: f64 = 0.006;

/// A pan drag across the whole viewport moves the focus by this much of the stand-off.
const PAN_PER_VIEWPORT: f64 = 1.2;

/// The minimap's side, in points.
const MINIMAP_SIDE: f32 = 190.0;

/// Bottom **left**. The event log is anchored bottom right with this same inset
/// (`panels.rs`), and two surfaces at one corner is one surface with the other underneath it.
const MINIMAP_MARGIN: egui::Vec2 = egui::vec2(12.0, -12.0);

/// Draw whichever surface is in force, and turn pointer input on it into actions.
///
/// The panel and the minimap are never both up: the minimap *is* the map when the panel is
/// closed, and two of the same thing at two sizes is the muddle `18-ui-style.md` is about.
pub fn draw(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    game: Res<Game>,
    mut map: ResMut<Map>,
    mut out: MessageWriter<Requested>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let open = ui_state.is_open(Panel::Map);
    map.shown = true;

    let response = match open {
        true => panel(ctx, &ui_state, &game, &mut map, &mut out),
        false => minimap(ctx, &mut map),
    };

    let Some((rect, response)) = response else {
        map.shown = false;
        return;
    };
    map.wanted = UVec2::new(
        (rect.width() * ctx.pixels_per_point()).round().max(1.0) as u32,
        (rect.height() * ctx.pixels_per_point()).round().max(1.0) as u32,
    );
    // The same gestures on both surfaces, from the same function, because two surfaces
    // showing one view that answer a drag differently is worse than either answer.
    read_input(ctx, &response, rect, &mut out);
    if !open && response.clicked() {
        // A click is not a drag — egui keeps them apart — so this is the one gesture the
        // minimap has that the panel does not.
        ask(&mut out, Action::OpenPanel(Panel::Map));
    }
}

/// The full surface: the map, and the controls for what it is showing.
fn panel(
    ctx: &egui::Context,
    ui_state: &Ui,
    game: &Game,
    map: &mut Map,
    out: &mut MessageWriter<Requested>,
) -> Option<(egui::Rect, egui::Response)> {
    let mut open = true;
    let mut answer = None;
    egui::Window::new("Map")
        .open(&mut open)
        .default_size([640.0, 520.0])
        .resizable(true)
        .show(ctx, |ui| {
            controls(ui, ui_state, game, out);
            ui.separator();
            let side = ui.available_size();
            answer = Some(image(ui, map, side, egui::Sense::click_and_drag()));
        });
    if !open {
        ask(out, Action::ClosePanel(Panel::Map));
    }
    answer
}

/// The corner surface. One gesture and no more.
///
/// It takes the same gestures the panel does, and a click on top of them, which opens the
/// panel.
///
/// **What that costs, stated because it is a real cost.** `crate::input`'s wheel and cursor
/// grab both stand down while egui wants the pointer, and this surface is always on screen —
/// so hovering the corner stops the ship's boom zooming, and a right-press begun here pans the
/// map instead of turning the view. A drag belongs to the widget it started on even after the
/// cursor leaves, which is right, and is also why the whole gesture is the map's.
///
/// Every panel in the interface already costs exactly this. The minimap is the only one that
/// is never closed, which is the whole of the difference and is a corner of 190 points.
fn minimap(ctx: &egui::Context, map: &mut Map) -> Option<(egui::Rect, egui::Response)> {
    let mut answer = None;
    egui::Area::new("minimap".into())
        // Middle, not Foreground: an open window has to cover this, not the other way round.
        .order(egui::Order::Middle)
        .anchor(egui::Align2::LEFT_BOTTOM, MINIMAP_MARGIN)
        .show(ctx, |ui| {
            let side = egui::vec2(MINIMAP_SIDE, MINIMAP_SIDE);
            answer = Some(image(ui, map, side, egui::Sense::click_and_drag()));
        });
    answer
}

/// Put the rendered texture on screen, with a sense so egui claims the pointer over it.
fn image(ui: &mut egui::Ui, map: &Map, size: egui::Vec2, sense: egui::Sense)
    -> (egui::Rect, egui::Response) {
    let size = egui::vec2(size.x.max(64.0), size.y.max(64.0));
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if let Some(texture) = map.texture {
        ui.painter().image(
            texture,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    (rect, response)
}

fn controls(ui: &mut egui::Ui, state: &Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    ui.horizontal(|ui| {
        ui.label("plane");
        for plane in [em_map::Plane::Ecliptic, em_map::Plane::Galactic] {
            if ui.selectable_label(state.map.plane == plane, plane.label()).clicked() {
                ask(out, Action::SetMapPlane(plane));
            }
        }
        ui.separator();
        ui.label(em_map::rings::label_m(state.map.orbit.distance_m()));
        ui.separator();
        // How old the picture is, which is the premise and belongs on every surface that
        // shows a position. See `13-client-shell.md`.
        ui.weak(match state.map.source {
            Source::Observed => "as seen from here",
            #[cfg(feature = "godview")]
            Source::God => "coordinate time, no light delay",
        });
    });

    #[cfg(feature = "godview")]
    ui.horizontal(|ui| {
        ui.label("source");
        for source in [Source::Observed, Source::God] {
            let allowed = source == Source::Observed || state.may_see_everything;
            let chosen = state.map.source == source;
            // A plain selectable label: `add_enabled` wrapping one reports clicks nobody made.
            // A refused source is shown grayed and says why rather than being absent, because
            // color is never the only signal.
            if !allowed {
                ui.weak(source.label()).on_hover_text("needs an administrative account");
                continue;
            }
            if ui.selectable_label(chosen, source.label()).clicked() {
                ask(out, Action::SetMapSource(source));
            }
        }
        if state.map.source == Source::God {
            ui.weak("— other ships are not drawn: their light has not arrived");
        }
    });

    ui.horizontal(|ui| {
        if ui.button("center on the ship").clicked() {
            ask(out, Action::FocusMap(None));
        }
        if let Some(system) = game.0.system.as_ref() {
            if ui.button(format!("center on {}", system.star_name)).clicked() {
                ask(out, Action::FocusMap(Some(em_map::ItemKey::from_id(
                    "star",
                    system.star.get(),
                ))));
            }
        }
    });
}

/// Drag and wheel over the full surface.
///
/// **Left** turns and **right** pans. Right is also the sky's look button, so a right-press
/// that starts on either map surface turns the map and not the view — the grab is asked for
/// once, on the press, and `grab_cursor` stands down while egui wants the pointer. That is a
/// drag belonging to the widget it began on, which is correct, and it is the price of the
/// second button.
fn read_input(
    ctx: &egui::Context,
    response: &egui::Response,
    rect: egui::Rect,
    out: &mut MessageWriter<Requested>,
) {
    if response.dragged() {
        let delta = response.drag_delta();
        // Right as well as middle, because the minimap is too small to reach for a modifier
        // over and a second button is the one thing a postage stamp has room for.
        let panning = response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
            || ctx.input(|i| i.modifiers.shift);
        if panning {
            let span = rect.width().max(rect.height()).max(1.0) as f64;
            ask(out, Action::PanMap {
                right: -delta.x as f64 / span * PAN_PER_VIEWPORT,
                ahead: delta.y as f64 / span * PAN_PER_VIEWPORT,
            });
        } else {
            ask(out, Action::TurnMap {
                azimuth: -delta.x as f64 * TURN_PER_POINT,
                elevation: delta.y as f64 * TURN_PER_POINT,
            });
        }
    }
    read_wheel_only(ctx, response, out);
}

fn read_wheel_only(
    ctx: &egui::Context,
    response: &egui::Response,
    out: &mut MessageWriter<Requested>,
) {
    if !response.hovered() {
        return;
    }
    // egui reports pixels, and `input::notches` is already the tested divider — it knows what
    // a trackpad does, which a number written here would have to learn again.
    let scrolled = ctx.input(|i| i.smooth_scroll_delta.y);
    if scrolled != 0.0 {
        let notches = crate::input::notches(
            bevy::input::mouse::MouseScrollUnit::Pixel,
            scrolled,
        );
        ask(out, Action::ZoomMap(notches));
    }
}
