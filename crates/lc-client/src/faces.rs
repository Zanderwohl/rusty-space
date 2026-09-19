//! The faces egui is set in, and the one place its font set is built.
//!
//! egui holds a single `FontDefinitions` and `set_fonts` replaces it whole, so two places
//! installing faces means the second silently undoes the first. Everything that adds one goes
//! through [`Faces::install`], which keeps the accumulated set and hands egui all of it.
//!
//! Fetched through the asset server rather than compiled in, for the reason
//! [`crate::library::FACES`] gives: the browser build takes them from the CDN, and a face in
//! the binary is bytes every player downloads whether or not anything sets type in it.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::library::FontFace;

/// Quantico: every readout, label and button.
pub const UI: &str = "interface";

/// Geo: the radio logs, and nothing else. What a ship said is set apart from the window that
/// is showing it — see [`crate::radio_panel`].
pub const RADIO: &str = "radio";

/// The faces asked for at startup, before anything is drawn.
const INTERFACE: &[(&str, &str, As)] = &[
    (UI, "fonts/Quantico-Regular.ttf", As::Interface),
    (RADIO, "fonts/Geo-Regular.ttf", As::Named),
];

/// How a face joins the set.
#[derive(Clone, Copy)]
pub enum As {
    /// The head of egui's proportional family: what everything is set in, with egui's own
    /// faces left under it for the glyphs it does not carry.
    Interface,
    /// A family of its own, with the interface family under it, so a glyph this face lacks is
    /// drawn rather than boxed. The tofu that avoids has already cost this interface a close
    /// button and a pair of arrows.
    Named,
    /// A family of its own and nothing under it: a page whose every glyph must come from the
    /// one face or not at all.
    Alone,
}

/// The set egui is drawing from, and what is still on its way.
#[derive(Resource)]
pub struct Faces {
    defs: egui::FontDefinitions,
    /// The [`As::Named`] families. Their fallback is the interface family, so a face that
    /// arrives after them has to put them back.
    named: Vec<String>,
    asked: Asked,
}

impl Default for Faces {
    fn default() -> Self {
        Self { defs: egui::FontDefinitions::default(), named: Vec::new(), asked: Asked::Unasked }
    }
}

enum Asked {
    Unasked,
    /// Parallel to [`INTERFACE`].
    Waiting(Vec<Handle<FontFace>>),
    Settled,
}

impl Faces {
    /// Adds faces to the set and hands egui the whole of it.
    pub fn install(
        &mut self,
        ctx: &egui::Context,
        added: impl IntoIterator<Item = (String, Vec<u8>, As)>,
    ) {
        for (name, bytes, how) in added {
            self.defs.font_data.insert(
                name.clone(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            match how {
                As::Interface => {
                    let list = self.defs.families.entry(egui::FontFamily::Proportional).or_default();
                    list.retain(|held| held != &name);
                    list.insert(0, name);
                }
                As::Named if !self.named.contains(&name) => self.named.push(name),
                As::Named => {}
                As::Alone => {
                    self.defs
                        .families
                        .insert(egui::FontFamily::Name(name.as_str().into()), vec![name]);
                }
            }
        }

        // Rebuilt from what the set holds *now* rather than appended to: a fallback list is a
        // list of keys into `font_data`, and epaint answers one that names a font it does not
        // have by panicking rather than by leaving a glyph out.
        let under = self
            .defs
            .families
            .get(&egui::FontFamily::Proportional)
            .cloned()
            .unwrap_or_default();
        for name in &self.named {
            let mut list = vec![name.clone()];
            list.extend(under.iter().filter(|held| *held != name).cloned());
            self.defs.families.insert(egui::FontFamily::Name(name.as_str().into()), list);
        }

        ctx.set_fonts(self.defs.clone());
    }
}

pub struct FacesPlugin;

impl Plugin for FacesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Faces>();
    }
}

/// Asks for the interface faces and installs them together once they have all landed.
///
/// Together, because a screen half in one face and half in another for the frame between them
/// arriving would reflow under whatever is being read. Until then egui draws in its own face,
/// which is a boot screen and a frame or two, not a flash anyone sees.
pub fn settle(
    mut contexts: EguiContexts,
    mut faces: ResMut<Faces>,
    assets: Res<AssetServer>,
    loaded: Res<Assets<FontFace>>,
) {
    use bevy::asset::LoadState;
    match &faces.asked {
        Asked::Settled => return,
        Asked::Unasked => {
            let asked = INTERFACE.iter().map(|(_, path, _)| assets.load(*path)).collect();
            faces.asked = Asked::Waiting(asked);
            return;
        }
        Asked::Waiting(asked) => {
            let settled = |handle| {
                matches!(
                    assets.get_load_state(handle),
                    Some(LoadState::Loaded) | Some(LoadState::Failed(_)) | None
                )
            };
            if !asked.iter().all(settled) {
                return;
            }
        }
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let ctx = ctx.clone();

    let Asked::Waiting(asked) = &faces.asked else { unreachable!() };
    let ready: Vec<(String, Vec<u8>, As)> = INTERFACE
        .iter()
        .zip(asked)
        .filter_map(|((name, _, how), handle)| {
            loaded.get(handle).map(|face| ((*name).to_owned(), face.0.clone(), *how))
        })
        .collect();
    let names: Vec<String> = ready.iter().map(|(name, _, _)| name.clone()).collect();
    match names.is_empty() {
        true => info!("no interface faces; egui keeps its own"),
        false => info!("the interface is set in {}", names.join(", ")),
    }
    faces.install(&ctx, ready);
    faces.asked = Asked::Settled;
}
