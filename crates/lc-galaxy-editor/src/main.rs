//! Galaxy Forge: the editor for the Lightcone galaxy. See `lightcone/docs/34-galaxy-authoring.md`.
//!
//! ```text
//! lc-galaxy-editor [timeline.ron] [--view region|sky] [--select <row>] [--pick <star>] [--chart hr|mass|age|feh]
//!                  [--shot <png> [--frames <n>]]
//! ```

mod charts;
mod doc;
mod palette;
mod params_panel;
mod stats_panel;
mod system_panel;
mod timeline_panel;
mod views;

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::window::WindowResolution;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use lc_galaxy::Timeline;

use doc::Doc;
use views::{View, ViewState};

const TITLE: &str = "Galaxy Forge";

#[derive(Default)]
struct Args {
    path: Option<PathBuf>,
    view: Option<View>,
    select: Option<usize>,
    pick: Option<usize>,
    chart: Option<charts::Plot>,
    shot: Option<PathBuf>,
    frames: u32,
}

fn args() -> Args {
    let mut a = Args { frames: 30, ..default() };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{arg} needs a value"));
        match arg.as_str() {
            "--view" => {
                a.view = Some(match value().as_str() {
                    "sky" => View::Sky,
                    _ => View::Region,
                })
            }
            "--select" => a.select = value().parse().ok(),
            "--pick" => a.pick = value().parse().ok(),
            "--chart" => a.chart = charts::Plot::named(&value()),
            "--shot" => a.shot = Some(value().into()),
            "--frames" => a.frames = value().parse().unwrap_or(30),
            _ => a.path = Some(arg.into()),
        }
    }
    a
}

fn default_galaxy() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../lc-galaxy/galaxies/phase-1.ron");
    path.canonicalize().unwrap_or(path)
}

#[derive(Resource)]
struct Ui {
    view: ViewState,
    charts: charts::Charts,
    dialog: Option<Dialog>,
    system: system_panel::Job,
}

enum Dialog {
    Open(String),
    SaveAs(String),
}

#[derive(Resource)]
struct Shot {
    path: Option<PathBuf>,
    frames: u32,
    pick: Option<usize>,
}

fn main() {
    let a = args();
    let path = a.path.clone().unwrap_or_else(default_galaxy);
    let mut doc = Doc::open(path.clone()).unwrap_or_else(|e| {
        let mut d = Doc::new(Timeline::new(1), Some(path));
        d.notice = Some(e);
        d
    });
    doc.selected = a.select;
    let mut view = ViewState::default();
    if let Some(v) = a.view {
        view.view = v;
    }
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: TITLE.into(), resolution: WindowResolution::new(1600, 1000), ..default() }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.04)))
        .insert_resource(doc)
        .insert_resource(Ui { view, charts: charts::Charts::showing(a.chart.unwrap_or_default()), dialog: None, system: default() })
        .insert_resource(Shot { path: a.shot, frames: a.frames, pick: a.pick })
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
        })
        .add_systems(EguiPrimaryContextPass, ui)
        .add_systems(Update, (title, shoot))
        .run();
}

fn title(doc: Res<Doc>, mut windows: Query<&mut Window>) {
    if !doc.is_changed() {
        return;
    }
    let name = doc.path.as_ref().and_then(|p| p.file_name()).map_or("untitled".into(), |n| n.to_string_lossy().into_owned());
    let want = format!("{TITLE} — {name}{}", if doc.dirty() { " *" } else { "" });
    for mut w in &mut windows {
        if w.title != want {
            w.title = want.clone();
        }
    }
}

fn ui(mut contexts: EguiContexts, mut doc: ResMut<Doc>, mut state: ResMut<Ui>, shot: Res<Shot>) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let doc = &mut *doc;
    let state = &mut *state;
    // Keys pressed while the window takes focus must not change a screenshot.
    if shot.path.is_some() {
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))));
    } else {
        shortcuts(ctx, doc);
    }
    doc.rebuild(ctx.input(|i| i.pointer.any_down()));

    let mut root = egui::Ui::new(
        ctx.clone(),
        "root".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );
    egui::Panel::top("menu").show(&mut root, |ui| menu(ui, doc, state));
    egui::Panel::bottom("status").show(&mut root, |ui| status(ui, doc));
    egui::Panel::left("timeline").resizable(true).default_size(320.0).show(&mut root, |ui| timeline_panel::show(ui, doc));
    egui::Panel::right("details").resizable(true).default_size(440.0).show(&mut root, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            params_panel::show(ui, doc);
            ui.add_space(8.0);
            egui::CollapsingHeader::new("Statistics").default_open(true).show(ui, |ui| stats_panel::statistics(ui, doc));
            egui::CollapsingHeader::new("Charts").default_open(true).show(ui, |ui| charts::show(ui, &mut state.charts, doc));
            egui::CollapsingHeader::new("Inspector").default_open(true).show(ui, |ui| stats_panel::inspector(ui, doc));
            egui::CollapsingHeader::new("System").default_open(true).show(ui, |ui| system_panel::show(ui, &mut state.system, doc));
        });
    });
    egui::CentralPanel::no_frame().show(&mut root, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| views::toolbar(ui, &mut state.view, doc));
        views::show(ui, &mut state.view, doc);
    });
    dialog(ctx, doc, state);

    // An undo step per gesture, not per frame of a drag.
    let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
    if !busy {
        doc.commit();
    }
}

fn shortcuts(ctx: &egui::Context, doc: &mut Doc) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let typing = ctx.memory(|m| m.focused().is_some());
    let redo = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
    let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
    ctx.input_mut(|i| {
        if i.consume_shortcut(&save) {
            if let Err(e) = doc.save() {
                doc.notice = Some(e);
            }
        }
        if typing {
            return;
        }
        if i.consume_shortcut(&redo) {
            doc.redo();
        } else if i.consume_shortcut(&undo) {
            doc.undo();
        }
    });
}

fn menu(ui: &mut egui::Ui, doc: &mut Doc, state: &mut Ui) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            if ui.button("New").clicked() {
                *doc = Doc::new(Timeline::new(1), None);
                ui.close();
            }
            if ui.button("Open…").clicked() {
                state.dialog = Some(Dialog::Open(doc.path.as_ref().map_or(String::new(), |p| p.display().to_string())));
                ui.close();
            }
            if ui.add_enabled(doc.path.is_some(), egui::Button::new("Save")).clicked() {
                if let Err(e) = doc.save() {
                    doc.notice = Some(e);
                }
                ui.close();
            }
            if ui.button("Save as…").clicked() {
                state.dialog = Some(Dialog::SaveAs(doc.path.as_ref().map_or(String::new(), |p| p.display().to_string())));
                ui.close();
            }
        });
        ui.menu_button("Edit", |ui| {
            if ui.add_enabled(doc.can_undo(), egui::Button::new("Undo")).clicked() {
                doc.undo();
                ui.close();
            }
            if ui.add_enabled(doc.can_redo(), egui::Button::new("Redo")).clicked() {
                doc.redo();
                ui.close();
            }
        });
        ui.menu_button("Bake", |ui| {
            ui.label(format!("seed {}", doc.timeline.seed));
            let mut seed = doc.timeline.seed;
            if ui.add(egui::DragValue::new(&mut seed).prefix("seed ")).changed() {
                doc.timeline.seed = seed;
            }
        });
    });
}

fn status(ui: &mut egui::Ui, doc: &Doc) {
    ui.horizontal(|ui| {
        let path = doc.path.as_ref().map_or("untitled".into(), |p| p.display().to_string());
        ui.label(format!("{path}{}", if doc.dirty() { " (modified)" } else { "" }));
        ui.separator();
        if let Some(r) = &doc.result {
            ui.label(format!("{} stars", doc.stars().len()));
            ui.separator();
            let problems = r.compiled.problems.len();
            if problems > 0 {
                ui.label(egui::RichText::new(format!("{problems} problem(s)")).color(palette::PROBLEM));
                ui.separator();
            }
        }
        if let Some(n) = &doc.notice {
            ui.label(n);
        }
    });
}

fn dialog(ctx: &egui::Context, doc: &mut Doc, state: &mut Ui) {
    let Some(d) = &mut state.dialog else { return };
    let (title, path) = match d {
        Dialog::Open(p) => ("Open", p),
        Dialog::SaveAs(p) => ("Save as", p),
    };
    let mut close = false;
    let mut go = false;
    egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        let r = ui.add(egui::TextEdit::singleline(path).desired_width(520.0));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            go = true;
        }
        ui.horizontal(|ui| {
            go |= ui.button(title).clicked();
            close |= ui.button("Cancel").clicked();
        });
    });
    if go {
        let path = PathBuf::from(path.trim());
        match d {
            Dialog::Open(_) => match Doc::open(path) {
                Ok(new) => *doc = new,
                Err(e) => doc.notice = Some(e),
            },
            Dialog::SaveAs(_) => {
                doc.path = Some(path);
                if let Err(e) = doc.save() {
                    doc.notice = Some(e);
                }
            }
        }
        close = true;
    }
    if close {
        state.dialog = None;
    }
}

/// `--shot`: photograph the window after `frames` frames and quit.
fn shoot(mut commands: Commands, mut shot: ResMut<Shot>, mut doc: ResMut<Doc>, mut frame: Local<u32>, mut exit: MessageWriter<AppExit>) {
    if let Some(i) = shot.pick.take() {
        doc.rebuild(false);
        doc.picked = doc.stars().get(i).map(|s| s.id);
    }
    let Some(path) = shot.path.clone() else { return };
    *frame += 1;
    if *frame == shot.frames {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
    }
    // The capture is asynchronous; quitting on the same frame loses the file.
    if *frame > shot.frames + 30 {
        exit.write(AppExit::Success);
    }
}
