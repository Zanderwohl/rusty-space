//! The File menu's actions: native pickers, and the unsaved-changes prompt before an action that
//! replaces or closes the document.

use std::path::PathBuf;
use std::pin::Pin;

use bevy::tasks::{IoTaskPool, Task, futures::check_ready};
use bevy::window::RawHandleWrapper;
use bevy_egui::egui;

use crate::doc::Doc;
use crate::palette;
use crate::views::ViewState;

/// An action that discards the open document.
#[derive(Clone, Copy)]
pub enum Action {
    New,
    Open,
    Quit,
}

enum Purpose {
    Open,
    SaveAs,
    SaveThen(Action),
}

enum Choice {
    Cancel,
    Discard,
    Save,
}

enum Step {
    Prompt(Action),
    Pick(Purpose, Task<Option<PathBuf>>),
}

#[derive(Default)]
pub struct Files {
    step: Option<Step>,
    /// Shown in the prompt, because its backdrop covers the status bar.
    failed: Option<String>,
    /// A close request that arrived while a picker was open.
    quit_queued: bool,
    /// The primary window, so a picker is modal to it on every platform.
    pub parent: Option<RawHandleWrapper>,
}

impl Files {
    pub fn busy(&self) -> bool {
        self.step.is_some()
    }

    /// Runs `action` at once if nothing would be lost, and otherwise asks first.
    /// A close request while busy replaces a prompt's action, or waits for the picker.
    pub fn request(&mut self, action: Action, doc: &mut Doc, view: &mut ViewState) -> bool {
        match (&mut self.step, action) {
            (None, _) => {}
            (Some(Step::Prompt(a)), Action::Quit) => *a = Action::Quit,
            (Some(Step::Pick(..)), Action::Quit) => self.quit_queued = true,
            (Some(_), _) => {}
        }
        if self.busy() {
            return false;
        }
        if doc.dirty() {
            self.step = Some(Step::Prompt(action));
            false
        } else {
            self.perform(action, doc, view)
        }
    }

    pub fn save(&mut self, doc: &mut Doc) {
        if self.busy() {
            return;
        }
        if doc.path.is_some() {
            if let Err(e) = doc.save() {
                doc.notice = Some(e);
            }
        } else {
            self.save_as(doc);
        }
    }

    pub fn save_as(&mut self, doc: &mut Doc) {
        if !self.busy() {
            self.step = Some(Step::Pick(Purpose::SaveAs, pick(false, doc, self.parent.as_ref())));
        }
    }

    /// Draws the prompt and collects a finished picker. True when the app should quit.
    ///
    /// Must run on the main thread: on macOS a picker opened from another thread waits on the
    /// main thread, which is waiting on this system.
    pub fn show(&mut self, ctx: &egui::Context, doc: &mut Doc, view: &mut ViewState) -> bool {
        let quit = self.advance(ctx, doc, view);
        if !quit && !matches!(self.step, Some(Step::Pick(..))) && std::mem::take(&mut self.quit_queued) {
            return self.request(Action::Quit, doc, view);
        }
        quit
    }

    fn advance(&mut self, ctx: &egui::Context, doc: &mut Doc, view: &mut ViewState) -> bool {
        match self.step.take() {
            None => false,
            Some(Step::Prompt(action)) => self.prompt(ctx, action, doc, view),
            Some(Step::Pick(purpose, mut task)) => match check_ready(&mut task) {
                None => {
                    self.step = Some(Step::Pick(purpose, task));
                    false
                }
                Some(picked) => self.picked(purpose, picked, doc, view),
            },
        }
    }

    fn prompt(&mut self, ctx: &egui::Context, action: Action, doc: &mut Doc, view: &mut ViewState) -> bool {
        let mut chosen = ctx.input(|i| i.key_pressed(egui::Key::Escape)).then_some(Choice::Cancel);
        egui::Modal::new("unsaved".into()).show(ctx, |ui| {
            ui.label("There are unsaved changes.");
            if let Some(e) = &self.failed {
                ui.label(egui::RichText::new(e).color(palette::PROBLEM));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    chosen = Some(Choice::Cancel);
                }
                if ui.button("Discard").clicked() {
                    chosen = Some(Choice::Discard);
                }
                if ui.button("Save").clicked() {
                    chosen = Some(Choice::Save);
                }
            });
        });
        if chosen.is_some() {
            self.failed = None;
        }
        match chosen {
            None => {
                self.step = Some(Step::Prompt(action));
                false
            }
            Some(Choice::Cancel) => false,
            Some(Choice::Discard) => self.perform(action, doc, view),
            Some(Choice::Save) if doc.path.is_some() => match doc.save() {
                Ok(()) => self.perform(action, doc, view),
                Err(e) => {
                    self.failed = Some(e);
                    self.step = Some(Step::Prompt(action));
                    false
                }
            },
            Some(Choice::Save) => {
                self.step = Some(Step::Pick(Purpose::SaveThen(action), pick(false, doc, self.parent.as_ref())));
                false
            }
        }
    }

    fn picked(&mut self, purpose: Purpose, picked: Option<PathBuf>, doc: &mut Doc, view: &mut ViewState) -> bool {
        match (purpose, picked) {
            (Purpose::Open, Some(path)) => match Doc::open(path) {
                Ok(new) => {
                    *doc = new;
                    view.forget_document();
                }
                Err(e) => doc.notice = Some(e),
            },
            (Purpose::SaveAs, Some(path)) => save_to(path, doc).unwrap_or_else(|e| doc.notice = Some(e)),
            (Purpose::SaveThen(action), Some(path)) => match save_to(path, doc) {
                Ok(()) => return self.perform(action, doc, view),
                Err(e) => {
                    self.failed = Some(e);
                    self.step = Some(Step::Prompt(action));
                }
            },
            (Purpose::SaveThen(action), None) => self.step = Some(Step::Prompt(action)),
            (Purpose::Open | Purpose::SaveAs, None) => {}
        }
        false
    }

    fn perform(&mut self, action: Action, doc: &mut Doc, view: &mut ViewState) -> bool {
        match action {
            Action::New => {
                *doc = Doc::new(lc_galaxy::Timeline::new(1), None);
                view.forget_document();
                false
            }
            Action::Open => {
                self.step = Some(Step::Pick(Purpose::Open, pick(true, doc, self.parent.as_ref())));
                false
            }
            Action::Quit => true,
        }
    }
}

/// Keeps the old path if the write fails, so a later Save does not aim at a file that was never made.
fn save_to(path: PathBuf, doc: &mut Doc) -> Result<(), String> {
    let old = doc.path.replace(path);
    doc.save().inspect_err(|_| doc.path = old)
}

fn pick(open: bool, doc: &Doc, parent: Option<&RawHandleWrapper>) -> Task<Option<PathBuf>> {
    let mut dialog = rfd::AsyncFileDialog::new().add_filter("Galaxy timeline", &["ron"]);
    if let Some(parent) = parent {
        // SAFETY: every caller is in the `ui` system, which runs on the main thread.
        dialog = dialog.set_parent(&unsafe { parent.get_handle() });
    }
    let near = doc.path.clone().unwrap_or_else(crate::default_galaxy);
    if let Some(dir) = near.parent() {
        dialog = dialog.set_directory(dir);
    }
    if !open && let Some(name) = doc.path.as_ref().and_then(|p| p.file_name()) {
        dialog = dialog.set_file_name(name.to_string_lossy());
    }
    // Made here, on the main thread, for the reason on `Files::show`.
    let chosen: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> =
        if open { Box::pin(dialog.set_title("Open").pick_file()) } else { Box::pin(dialog.set_title("Save as").save_file()) };
    IoTaskPool::get().spawn(async move { chosen.await.map(|h| h.path().to_path_buf()) })
}
