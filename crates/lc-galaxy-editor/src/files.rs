//! The File menu's actions: native pickers, and the unsaved-changes prompt before an action that
//! replaces or closes the document.

use std::path::PathBuf;
use std::pin::Pin;

use bevy::tasks::{IoTaskPool, Task, futures::check_ready};
use bevy_egui::egui;

use crate::doc::Doc;
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
    /// Saving for the first time on the way to an action; cancelling returns to the prompt.
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
}

impl Files {
    pub fn busy(&self) -> bool {
        self.step.is_some()
    }

    /// Runs `action` at once if nothing would be lost, and otherwise asks first.
    pub fn request(&mut self, action: Action, doc: &mut Doc, view: &mut ViewState) -> bool {
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
            self.step = Some(Step::Pick(Purpose::SaveAs, pick(false, doc)));
        }
    }

    /// Draws the prompt and collects a finished picker. True when the app should quit.
    ///
    /// Must run on the main thread: on macOS a picker opened from another thread waits on the
    /// main thread, which is waiting on this system.
    pub fn show(&mut self, ctx: &egui::Context, doc: &mut Doc, view: &mut ViewState) -> bool {
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
                    doc.notice = Some(e);
                    self.step = Some(Step::Prompt(action));
                    false
                }
            },
            Some(Choice::Save) => {
                self.step = Some(Step::Pick(Purpose::SaveThen(action), pick(false, doc)));
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
                    doc.notice = Some(e);
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
                self.step = Some(Step::Pick(Purpose::Open, pick(true, doc)));
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

/// Starts in the document's folder, or the bundled galaxies for a new document.
fn pick(open: bool, doc: &Doc) -> Task<Option<PathBuf>> {
    let mut dialog = rfd::AsyncFileDialog::new().add_filter("Galaxy timeline", &["ron"]);
    let near = doc.path.clone().unwrap_or_else(crate::default_galaxy);
    if let Some(dir) = near.parent() {
        dialog = dialog.set_directory(dir);
    }
    if !open && let Some(name) = doc.path.as_ref().and_then(|p| p.file_name()) {
        dialog = dialog.set_file_name(name.to_string_lossy());
    }
    // The future is made here, on the main thread; only the waiting moves to the pool.
    let chosen: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> =
        if open { Box::pin(dialog.set_title("Open").pick_file()) } else { Box::pin(dialog.set_title("Save as").save_file()) };
    IoTaskPool::get().spawn(async move { chosen.await.map(|h| h.path().to_path_buf()) })
}
