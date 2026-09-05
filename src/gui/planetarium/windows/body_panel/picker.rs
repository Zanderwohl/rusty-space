//! Choosing a body, shared by the Viewer and the Editor.
//!
//! A flat alphabetical list is useless once a system has barycentres and moons in it, so
//! the tree follows the primary hierarchy the arena already derives. Typing switches to a
//! flat search, because when you know the name you do not want to walk the tree.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::egui::{self, Ui};
use em_sim::id::BodyIndex;
use em_sim::system::System;

use crate::gui::planetarium::FocusedBodyState;

#[derive(Resource, Default)]
pub struct BodyPickerState {
    pub search: String,
}

/// Children by parent slot, plus the roots, both sorted by display name.
struct Hierarchy {
    roots: Vec<BodyIndex>,
    children: HashMap<usize, Vec<BodyIndex>>,
}

fn hierarchy(system: &System) -> Hierarchy {
    let mut roots = Vec::new();
    let mut children: HashMap<usize, Vec<BodyIndex>> = HashMap::new();
    for i in system.indices() {
        match system.parent(i) {
            Some(parent) => children.entry(parent.get()).or_default().push(i),
            None => roots.push(i),
        }
    }
    let by_name = |system: &System, list: &mut Vec<BodyIndex>| {
        list.sort_by(|a, b| system.info(*a).display_name().cmp(system.info(*b).display_name()));
    };
    by_name(system, &mut roots);
    for list in children.values_mut() {
        by_name(system, list);
    }
    Hierarchy { roots, children }
}

/// The body chooser. Returns true when the selection changed.
pub fn body_picker(
    ui: &mut Ui,
    id_source: &str,
    system: &System,
    focused: &mut FocusedBodyState,
    picker: &mut BodyPickerState,
) -> bool {
    let selected = focused
        .current_body_id
        .as_deref()
        .and_then(|id| system.by_name(id));

    let header = match selected {
        Some(i) => system.info(i).display_name().to_string(),
        None => "Choose a body".to_string(),
    };

    let mut changed = false;
    egui::CollapsingHeader::new(header)
        .id_salt(format!("{id_source}_picker"))
        .default_open(selected.is_none())
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut picker.search)
                        .hint_text("Filter")
                        .desired_width(160.0),
                );
                if ui.small_button("×").on_hover_text("Clear the filter").clicked() {
                    picker.search.clear();
                }
            });

            egui::ScrollArea::vertical()
                .max_height(220.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if picker.search.trim().is_empty() {
                        let tree = hierarchy(system);
                        for root in &tree.roots {
                            changed |= node(ui, id_source, system, &tree, *root, 0, focused);
                        }
                    } else {
                        changed |= search_results(ui, system, &picker.search, focused);
                    }
                });
        });

    changed
}

fn search_results(ui: &mut Ui, system: &System, needle: &str, focused: &mut FocusedBodyState) -> bool {
    let needle = needle.trim().to_lowercase();
    let mut hits: Vec<BodyIndex> = system
        .indices()
        .filter(|i| {
            let info = system.info(*i);
            info.display_name().to_lowercase().contains(&needle)
                || info.id.to_lowercase().contains(&needle)
                || info.designation.as_deref().is_some_and(|d| d.to_lowercase().contains(&needle))
                || info.tags.iter().any(|t| t.to_lowercase().contains(&needle))
        })
        .collect();
    hits.sort_by(|a, b| system.info(*a).display_name().cmp(system.info(*b).display_name()));

    if hits.is_empty() {
        ui.weak("No match");
        return false;
    }

    let mut changed = false;
    for i in hits {
        changed |= entry(ui, system, i, focused);
    }
    changed
}

/// One tree node: the body itself, then its satellites underneath.
fn node(
    ui: &mut Ui,
    id_source: &str,
    system: &System,
    tree: &Hierarchy,
    index: BodyIndex,
    depth: usize,
    focused: &mut FocusedBodyState,
) -> bool {
    let Some(children) = tree.children.get(&index.get()) else {
        return entry(ui, system, index, focused);
    };

    let mut changed = false;
    let id = ui.make_persistent_id((id_source, "node", index.get()));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth == 0)
        .show_header(ui, |ui| {
            changed |= entry(ui, system, index, focused);
            ui.weak(format!("{}", children.len()))
                .on_hover_text("Bodies orbiting this one");
        })
        .body(|ui| {
            for child in children {
                changed |= node(ui, id_source, system, tree, *child, depth + 1, focused);
            }
        });
    changed
}

/// A selectable body row. Hover carries the identifiers the label leaves out.
fn entry(ui: &mut Ui, system: &System, index: BodyIndex, focused: &mut FocusedBodyState) -> bool {
    let info = system.info(index);
    let is_selected = focused.is_focused(&info.id);
    let mut hover = format!("ID: {}", info.id);
    if let Some(designation) = &info.designation {
        hover.push_str(&format!("\nDesignation: {designation}"));
    }
    if !info.tags.is_empty() {
        hover.push_str(&format!("\n{}", info.tags.join(", ")));
    }

    let response = ui
        .selectable_label(is_selected, info.display_name())
        .on_hover_text(hover);

    if response.clicked() && !is_selected {
        focused.current_body_id = Some(info.id.clone());
        return true;
    }
    false
}

/// `Sol › Earth › Luna`, with every ancestor a link back up the chain.
pub fn breadcrumb(ui: &mut Ui, system: &System, index: BodyIndex, focused: &mut FocusedBodyState) -> bool {
    let mut chain = vec![index];
    let mut cursor = index;
    while let Some(parent) = system.parent(cursor) {
        if chain.contains(&parent) {
            break; // a cycle should be impossible, but never hang the UI over it
        }
        chain.push(parent);
        cursor = parent;
    }
    chain.reverse();

    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (position, step) in chain.iter().enumerate() {
            if position > 0 {
                ui.weak("›");
            }
            let info = system.info(*step);
            if *step == index {
                ui.strong(info.display_name());
            } else if ui.link(info.display_name()).clicked() {
                focused.current_body_id = Some(info.id.clone());
                changed = true;
            }
        }
    });
    changed
}
