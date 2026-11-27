//! Galaxy Forge: the editor for the Lightcone galaxy. See `lightcone/docs/34-galaxy-authoring.md`.
//!
//! ```text
//! lc-galaxy-editor [timeline.ron] [--view region|sky|picture] [--camera +x|-x|+y|-y|+z|-z] [--select <row>] [--pick <star>] [--chart hr|mass|age|feh|ocean-feh|ocean-mass]
//!                  [--shot <png> [--frames <n>]]
//! ```

mod census;
mod charts;
mod compare;
mod doc;
mod files;
mod palette;
mod params_panel;
mod picture_view;
mod stats_panel;
mod system_panel;
mod timeline_panel;
mod views;

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::ecs::system::NonSendMarker;
use bevy::window::{PrimaryWindow, RawHandleWrapper, WindowCloseRequested, WindowResolution};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use lc_galaxy::Timeline;

use doc::Doc;
use views::{View, ViewState};

const TITLE: &str = "Galaxy Forge";

#[derive(Default)]
struct Args {
    path: Option<PathBuf>,
    view: Option<View>,
    camera: Option<views::Side>,
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
                    "picture" => View::Picture,
                    _ => View::Region,
                })
            }
            "--camera" => {
                let name = value();
                a.camera = Some(views::Side::named(&name).unwrap_or_else(|| panic!("--camera {name}: not one of +x, -x, +y, -y, +z, -z")));
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

pub(crate) fn default_galaxy() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../lc-galaxy/galaxies/phase-1.ron");
    path.canonicalize().unwrap_or(path)
}

#[derive(Resource)]
struct Ui {
    view: ViewState,
    charts: charts::Charts,
    files: files::Files,
    system: system_panel::SystemView,
    census: census::CensusJob,
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
    if let Some(p) = a.camera {
        view.camera.look_from(p);
    }
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: TITLE.into(), resolution: WindowResolution::new(1600, 1000), ..default() }),
            // Closing, by the button or ⌘Q, asks first when there is unsaved work.
            close_when_requested: false,
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_plugins(lc_icon::AppIconPlugin(lc_icon::GALAXY))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.04)))
        .insert_resource(doc)
        .insert_resource(Ui { view, charts: charts::Charts::showing(a.chart.unwrap_or_default()), files: default(), system: default(), census: default() })
        .insert_resource(Shot { path: a.shot, frames: a.frames, pick: a.pick })
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
        })
        .add_systems(EguiPrimaryContextPass, ui)
        .add_systems(Update, (title, shoot));
    #[cfg(target_os = "macos")]
    app.add_systems(Update, quit_asks_first);
    app.run();
}

/// winit's app menu sends `terminate:`, which exits with no close request. Sent as
/// `performClose:` it closes the key window instead, so Quit asks first like the close button.
#[cfg(target_os = "macos")]
fn quit_asks_first(mut done: Local<bool>, _main: NonSendMarker) {
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::NSApplication;
    if *done {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() else { return };
    for top in menu.itemArray() {
        let Some(sub) = top.submenu() else { continue };
        for item in sub.itemArray() {
            if item.action() == Some(sel!(terminate:)) {
                // SAFETY: `performClose:` takes the sender, as `terminate:` does.
                unsafe { item.setAction(Some(sel!(performClose:))) };
                *done = true;
            }
        }
    }
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

/// On the main thread for the native file pickers; see [`files::Files::show`].
fn ui(
    mut contexts: EguiContexts,
    mut doc: ResMut<Doc>,
    mut state: ResMut<Ui>,
    shot: Res<Shot>,
    mut close: MessageReader<WindowCloseRequested>,
    mut exit: MessageWriter<AppExit>,
    window: Query<&RawHandleWrapper, With<PrimaryWindow>>,
    _main: NonSendMarker,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let doc = &mut *doc;
    let state = &mut *state;
    state.files.parent = window.single().ok().cloned();
    // Keys pressed while the window takes focus must not change a screenshot.
    if shot.path.is_some() {
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))));
    } else {
        shortcuts(ctx, doc, state);
    }
    let mut quit = false;
    if close.read().count() > 0 {
        quit |= state.files.request(files::Action::Quit, doc, &mut state.view);
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
            egui::CollapsingHeader::new("Statistics").default_open(true).show(ui, |ui| {
                stats_panel::statistics(ui, doc);
                state.census.update(doc);
                stats_panel::systems(ui, &state.census, doc);
            });
            egui::CollapsingHeader::new("Charts").default_open(true).show(ui, |ui| {
                state.census.update(doc);
                charts::show(ui, &mut state.charts, doc, &state.census);
            });
            egui::CollapsingHeader::new("Inspector").default_open(true).show(ui, |ui| stats_panel::inspector(ui, doc));
            egui::CollapsingHeader::new("System").default_open(true).show(ui, |ui| system_panel::show(ui, &mut state.system, doc));
        });
    });
    egui::CentralPanel::no_frame().show(&mut root, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| views::toolbar(ui, &mut state.view, doc));
        views::show(ui, &mut state.view, doc);
    });
    quit |= state.files.show(ctx, doc, &mut state.view);
    if quit {
        exit.write(AppExit::Success);
    }

    // An undo step per gesture, not per frame of a drag.
    let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
    if !busy {
        doc.commit();
    }
}

fn shortcuts(ctx: &egui::Context, doc: &mut Doc, state: &mut Ui) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let typing = ctx.memory(|m| m.focused().is_some());
    let redo = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
    let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
    ctx.input_mut(|i| {
        if i.consume_shortcut(&save) {
            state.files.save(doc);
        }
        if typing || state.files.busy() {
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
            ui.add_enabled_ui(!state.files.busy(), |ui| {
                if ui.button("New").clicked() {
                    state.files.request(files::Action::New, doc, &mut state.view);
                    ui.close();
                }
                if ui.button("Open…").clicked() {
                    state.files.request(files::Action::Open, doc, &mut state.view);
                    ui.close();
                }
                if ui.button("Save").clicked() {
                    state.files.save(doc);
                    ui.close();
                }
                if ui.button("Save as…").clicked() {
                    state.files.save_as(doc);
                    ui.close();
                }
            });
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
