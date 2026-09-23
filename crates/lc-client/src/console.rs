//! The console: a line typed after `/`, sent to the shard as typed, and what came back.
//!
//! **Nothing here parses a command.** A line is text on the wire and the shard's to read — see
//! `lc_server::command` — so a client can neither know what commands exist nor send one the
//! shard would not have parsed. `help` is how a player finds out, and it answers per level.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::action::Action;
use crate::app::Ui;
use crate::input::Requested;
use crate::panels::ask;
use crate::ui::Panel;

/// How many lines the console keeps, answers included.
const KEPT: usize = 200;

/// Everything typed this session, and what each became.
#[derive(Default)]
pub struct Console {
    entries: Vec<Entry>,
    next_seq: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub seq: u32,
    pub line: String,
    pub answer: Answer,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Waiting,
    Done(String),
    Failed(String),
}

impl Console {
    /// Record a line going out, and the number its answer will carry.
    pub fn sent(&mut self, line: String) -> u32 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.push(Entry { seq, line, answer: Answer::Waiting });
        seq
    }

    /// Record a line that could not go out, and why.
    pub fn unsent(&mut self, line: String, why: &str) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.push(Entry { seq, line, answer: Answer::Failed(why.into()) });
    }

    /// The shard's answer to line `seq`. An answer to a line this console no longer holds is
    /// dropped: it scrolled away, or it is for a previous connection.
    pub fn answered(&mut self, seq: u32, ok: bool, text: String) {
        let Some(entry) = self.entries.iter_mut().rev().find(|e| e.seq == seq) else { return };
        entry.answer = if ok { Answer::Done(text) } else { Answer::Failed(text) };
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The line typed `back` lines ago, 1 being the last, skipping repeats.
    pub fn recall(&self, back: usize) -> Option<&str> {
        let mut lines: Vec<&str> = self.entries.iter().map(|e| e.line.as_str()).collect();
        lines.dedup();
        lines.len().checked_sub(back).and_then(|at| lines.get(at)).copied()
    }

    fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
        let excess = self.entries.len().saturating_sub(KEPT);
        self.entries.drain(..excess);
    }
}

/// What is being typed, and where in the history the arrows have got to.
#[derive(Default)]
pub struct Draft {
    text: String,
    back: usize,
    /// Focus the field on the next draw: on opening, and after each line is sent.
    focus: bool,
    was_open: bool,
}

/// The console window. Hangs from the top of the screen, over everything, while it is open.
pub fn draw(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    uplink: Res<crate::uplink::Uplink>,
    mut out: MessageWriter<Requested>,
    mut draft: Local<Draft>,
) {
    let open = ui_state.is_open(Panel::Console);
    if open && !draft.was_open {
        draft.focus = true;
        draft.back = 0;
    }
    draft.was_open = open;
    if !open {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    // Opaque: it is drawn over whatever panels are open, and two translucent surfaces in one
    // place read as one muddled one. See `lightcone/docs/18-ui-style.md`.
    let frame = egui::Frame::window(&ctx.global_style()).fill(ctx.global_style().visuals.window_fill.to_opaque());
    let width = (ctx.content_rect().width() - 32.0).clamp(240.0, 640.0);
    egui::Window::new("Console")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_TOP, [0.0, 72.0])
        .min_width(width)
        .max_width(width)
        .order(egui::Order::Foreground)
        .frame(frame)
        .show(ctx, |ui| {
            history(ui, uplink.console.entries());
            field(ui, &uplink.console, &mut draft, &mut out);
        });
}

fn history(ui: &mut egui::Ui, entries: &[Entry]) {
    if entries.is_empty() {
        ui.weak("Type help and press Enter.");
        return;
    }
    egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
        for entry in entries {
            ui.label(egui::RichText::new(format!("/{}", entry.line)).monospace().strong());
            match &entry.answer {
                Answer::Waiting => {
                    ui.weak("…");
                }
                Answer::Done(text) => {
                    ui.label(egui::RichText::new(text).monospace());
                }
                Answer::Failed(text) => {
                    ui.label(egui::RichText::new(text).monospace().color(egui::Color32::from_rgb(235, 110, 100)));
                }
            }
        }
    });
    ui.separator();
}

fn field(ui: &mut egui::Ui, console: &Console, draft: &mut Draft, out: &mut MessageWriter<Requested>) {
    // Taken before the field sees them, or they would move its cursor as well.
    let (up, down, escape) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    if escape {
        ask(out, Action::ClosePanel(Panel::Console));
        return;
    }
    if up && console.recall(draft.back + 1).is_some() {
        draft.back += 1;
    }
    if down {
        draft.back = draft.back.saturating_sub(1);
    }
    if up || down {
        draft.text = console.recall(draft.back).unwrap_or_default().to_string();
    }

    let response = ui.add(
        egui::TextEdit::singleline(&mut draft.text)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace)
            .hint_text("/help"),
    );
    if std::mem::take(&mut draft.focus) {
        response.request_focus();
    }
    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        let line = std::mem::take(&mut draft.text);
        if !line.trim().is_empty() {
            ask(out, Action::RunCommand(line.trim().to_string()));
        }
        draft.back = 0;
        draft.focus = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_lands_on_the_line_it_answers() {
        let mut console = Console::default();
        let first = console.sent("help".into());
        let second = console.sent("where".into());
        console.answered(second, true, "star 0x1".into());
        console.answered(first, false, "no".into());
        assert_eq!(console.entries()[0].answer, Answer::Failed("no".into()));
        assert_eq!(console.entries()[1].answer, Answer::Done("star 0x1".into()));
        // One for a line it never sent changes nothing.
        console.answered(99, true, "?".into());
        assert_eq!(console.entries().len(), 2);
    }

    #[test]
    fn recall_walks_back_through_what_was_typed_skipping_repeats() {
        let mut console = Console::default();
        for line in ["help", "where", "where", "teleport 1"] {
            console.sent(line.into());
        }
        assert_eq!(console.recall(1), Some("teleport 1"));
        assert_eq!(console.recall(2), Some("where"));
        assert_eq!(console.recall(3), Some("help"));
        assert_eq!(console.recall(4), None);
        assert_eq!(console.recall(0), None);
    }

    #[test]
    fn the_history_is_bounded() {
        let mut console = Console::default();
        for n in 0..(KEPT + 10) {
            console.sent(format!("help {n}"));
        }
        assert_eq!(console.entries().len(), KEPT);
        assert_eq!(console.recall(1), Some(format!("help {}", KEPT + 9).as_str()));
    }
}
