//! Launching a ship, for an account that has none: new, or its last ship collapsed.
//!
//! A collapse is watched from the third-person camera for [`WATCH_S`] before the launch screen
//! replaces the world. The shard sends nothing more about the world until a launch is accepted,
//! and the [`Outbound::Welcome`](lc_proto::Outbound::Welcome) that answers one returns the game.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::app::{AppState, Ui};
use crate::uplink::{State, Uplink};

/// Real seconds a collapse is watched before the launch screen.
pub const WATCH_S: f64 = 10.0;

#[derive(Resource, Default)]
pub struct Launch {
    pub name: String,
    /// The name sent and not yet answered.
    pub sent: Option<String>,
    focused: bool,
}

pub struct OnboardingPlugin;

impl Plugin for OnboardingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Launch>()
            .add_systems(Update, leave_for_launch.in_set(crate::app::Stage::Act).run_if(in_state(AppState::InGame)))
            .add_systems(Update, board.in_set(crate::app::Stage::Act).run_if(in_state(AppState::Onboarding)))
            .add_systems(OnEnter(AppState::Onboarding), (crate::input::release_cursor, reset))
            .add_systems(EguiPrimaryContextPass, draw.run_if(in_state(AppState::Onboarding)));
    }
}

/// Whether the game should give way to the launch screen, `now_s` being real seconds: at once
/// for an account with no ship, and [`WATCH_S`] after a collapse was first shown.
pub fn due(uplink: &Uplink, now_s: f64) -> bool {
    if uplink.state != State::Unlaunched {
        return false;
    }
    match &uplink.fallen {
        None => true,
        Some(fallen) => fallen.shown_s.is_some_and(|shown| now_s - shown >= WATCH_S),
    }
}

fn leave_for_launch(time: Res<Time>, mut uplink: ResMut<Uplink>, mut ui: ResMut<Ui>, mut next: ResMut<NextState<AppState>>) {
    let now_s = time.elapsed_secs_f64();
    if let Some(fallen) = uplink.fallen.as_mut() {
        fallen.shown_s.get_or_insert(now_s);
    }
    if due(&uplink, now_s) {
        ui.screen = crate::ui::Screen::Onboarding;
        next.set(AppState::Onboarding);
    }
}

/// None of what the old ship saw, said or was fitted with passes to the next.
fn reset(mut launch: ResMut<Launch>, mut uplink: ResMut<Uplink>, mut wrecks: ResMut<crate::field::Wrecks>) {
    *launch = Launch::default();
    *wrecks = default();
    let uplink = &mut *uplink;
    uplink.fallen = None;
    uplink.launch_refused = None;
    uplink.seen.clear();
    uplink.contacts.clear();
    uplink.labels.clear();
    uplink.chat = default();
    uplink.incoming = default();
    uplink.beams = default();
    uplink.applied = None;
    uplink.fitting = None;
    uplink.field = None;
    uplink.hull = None;
}

fn board(uplink: Res<Uplink>, mut ui: ResMut<Ui>, mut next: ResMut<NextState<AppState>>) {
    if uplink.joined().is_some() {
        ui.screen = crate::ui::Screen::InGame;
        next.set(AppState::InGame);
    }
}

/// What the launch screen says in place of its controls when no launch can reach the shard.
fn lost(state: &State) -> Option<String> {
    crate::uplink::note(state, None).filter(|(note, _)| *note == crate::uplink::Note::Wrong).map(|(_, said)| said)
}

fn draw(mut contexts: EguiContexts, mut launch: ResMut<Launch>, mut uplink: ResMut<Uplink>) {
    let Ok(context) = contexts.ctx_mut() else { return };
    let color = crate::panels::color;
    // A refusal answers the launch, and a lost link never will.
    let lost = lost(&uplink.state);
    if uplink.launch_refused.is_some() || lost.is_some() {
        launch.sent = None;
    }
    let mut send = false;
    let mut root = crate::panels::viewport_ui(context);
    egui::CentralPanel::default()
        .frame(egui::Frame { fill: color(em_ui::vfd::PANEL_BG.with_alpha(1.0_f32)), ..egui::Frame::NONE })
        .show(&mut root, |_| {});
    egui::Window::new("Launch")
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .frame(egui::Frame {
            fill: color(em_ui::vfd::PANEL_BG.with_alpha(1.0_f32)),
            stroke: egui::Stroke::new(1.0_f32, color(em_ui::vfd::BUTTON_BORDER)),
            inner_margin: egui::Margin::same(14),
            ..egui::Frame::NONE
        })
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(context, |ui| {
            ui.visuals_mut().override_text_color = Some(color(em_ui::vfd::TEXT));
            ui.visuals_mut().widgets.inactive.bg_fill = color(em_ui::vfd::BUTTON_BG);
            ui.visuals_mut().widgets.hovered.bg_fill = color(em_ui::vfd::BUTTON_HOVER);
            ui.visuals_mut().widgets.active.bg_fill = color(em_ui::vfd::BUTTON_HOVER);
            ui.visuals_mut().extreme_bg_color = color(em_ui::vfd::FIELD_BG);
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, color(em_ui::vfd::TEXT_DIM));
            ui.set_min_width(320.0);
            let waiting = launch.sent.is_some() || lost.is_some();
            let field = ui.horizontal(|ui| {
                ui.label("Ship name");
                ui.add_enabled(!waiting, egui::TextEdit::singleline(&mut launch.name).char_limit(lc_proto::NAME_LIMIT))
            });
            let field = field.inner;
            if !launch.focused {
                field.request_focus();
                launch.focused = true;
            }
            if let Some(lost) = &lost {
                ui.colored_label(color(em_ui::vfd::TEXT_DIM), lost);
            } else if let Some(why) = uplink.launch_refused {
                let words = match why {
                    lc_proto::LaunchRefusal::Name => "The shard will not take that name.",
                    lc_proto::LaunchRefusal::Flying => "This account already flies a ship.",
                };
                ui.colored_label(color(em_ui::vfd::TEXT_DIM), words);
            }
            ui.separator();
            let ready = !waiting && lc_proto::ship_name(&launch.name).is_some();
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            send = ui.add_enabled(ready, egui::Button::new("Launch")).clicked() || (ready && entered);
        });
    if send && let Some(name) = lc_proto::ship_name(&launch.name).map(str::to_owned) {
        uplink.launch_refused = None;
        uplink.say(lc_proto::Inbound::Launch { name: name.clone() });
        launch.sent = Some(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uplink::Fallen;
    use lc_proto::ShipId;

    fn fallen() -> Fallen {
        Fallen { ship_id: ShipId(4), at_t: 1, released_j: 1.0, at_ly: glam::DVec3::ZERO, shown_s: None }
    }

    #[test]
    fn an_account_with_no_ship_launches_at_once() {
        let mut uplink = Uplink::default();
        uplink.state = State::Unlaunched;
        assert!(due(&uplink, 0.0));
    }

    #[test]
    fn a_collapse_is_watched_before_the_launch_screen() {
        let mut uplink = Uplink::default();
        uplink.state = State::Unlaunched;
        uplink.fallen = Some(fallen());
        assert!(!due(&uplink, 100.0), "not yet shown");
        uplink.fallen.as_mut().unwrap().shown_s = Some(100.0);
        assert!(!due(&uplink, 100.0 + WATCH_S - 0.1));
        assert!(due(&uplink, 100.0 + WATCH_S));
    }

    #[test]
    fn a_lost_link_is_said_on_the_launch_screen() {
        assert_eq!(lost(&State::Unlaunched), None);
        assert!(lost(&State::Lost("socket closed".into())).is_some_and(|said| said.contains("socket closed")));
    }

    #[test]
    fn a_flying_ship_never_launches() {
        assert!(!due(&Uplink::default(), 1.0e9));
    }
}
