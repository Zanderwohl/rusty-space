//! The faces egui is set in, how it rasterises them, and the one place its font set is built.
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

/// Where Quantico comes from. Public because the main menu is Bevy UI rather than egui and
/// loads its own copy of the same file — Bevy UI takes a `Handle<Font>` and has no font set
/// to name a family in. One constant so the two cannot drift onto different cuts.
pub const UI_FILE: &str = "fonts/Quantico-Regular.ttf";

/// Geo: the radio logs, and nothing else. What a ship said is set apart from the window that
/// is showing it — see [`crate::radio_panel`].
pub const RADIO: &str = "radio";

/// What is asked for at startup, before anything is drawn. The reader asks for its own when a
/// book is first opened.
const STARTUP: &[(&str, &str, As)] = &[
    (UI, UI_FILE, As::Interface),
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
    asked: Asked,
}

impl Default for Faces {
    fn default() -> Self {
        Self {
            defs: egui::FontDefinitions::default(),
            asked: Asked::Unasked,
        }
    }
}

enum Asked {
    Unasked,
    /// Parallel to [`STARTUP`].
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
        let added: Vec<(String, As)> = added
            .into_iter()
            .map(|(name, bytes, how)| {
                self.defs.font_data.insert(
                    name.clone(),
                    std::sync::Arc::new(egui::FontData::from_owned(bytes)),
                );
                (name, how)
            })
            .collect();

        // The interface head first, and in a pass of its own: a named face installed in the
        // same call falls back through the proportional list, and this is what puts the
        // interface face into it.
        for (name, _) in added.iter().filter(|(_, how)| matches!(how, As::Interface)) {
            let list = self
                .defs
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default();
            list.retain(|held| held != name);
            list.insert(0, name.clone());
        }
        for (name, how) in &added {
            // A fallback list is a list of keys into `font_data`, and epaint answers one that
            // names a font it does not hold by panicking rather than by leaving a glyph out.
            // So it is taken from the set as it stands and never written ahead of a face.
            let mut list = vec![name.clone()];
            match how {
                As::Interface => continue,
                As::Named => {
                    let under = self.defs.families.get(&egui::FontFamily::Proportional);
                    list.extend(under.into_iter().flatten().filter(|h| *h != name).cloned());
                }
                As::Alone => {}
            }
            self.defs
                .families
                .insert(egui::FontFamily::Name(name.as_str().into()), list);
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

/// Hinting off, everywhere.
///
/// epaint rounds glyph coordinates to the pixel grid by default. macOS has not hinted since
/// CoreText dropped it, so every browser on the platform draws unhinted — and a face chosen by
/// looking at it in a browser was chosen unhinted.
///
/// **It changes almost nothing for the faces this client ships**, and that is worth stating so
/// nobody re-derives it: hinting is a program in the font, and Quantico and Faustina carry
/// none — no `fpgm`, no `cvt`, a seven-byte `prep` stub. Geo carries a token one. What it does
/// reach is egui's own Ubuntu-Light, which is properly hinted and which sits under the
/// interface family as the fallback for every glyph Quantico lacks. So this is here to keep
/// that one face from being the odd crunchy word in an unhinted line.
///
/// Context-wide, and unavoidably so: egui reads this off the **global** style, so the reader
/// cannot have an answer of its own. It is also read when a face is *constructed* rather than
/// per pass, so it has to be set before anything calls `set_fonts` — which is why this runs
/// ahead of [`settle`] rather than inside it.
pub fn unhint(mut contexts: EguiContexts, mut done: Local<bool>) {
    if *done {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    ctx.all_styles_mut(|style| style.visuals.text_options.font_hinting = false);
    *done = true;
}

/// Asks for the startup faces and installs them together once they have all landed.
///
/// Together, because a surface half in one face and half in another for the frame between them
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
            let asked = STARTUP
                .iter()
                .map(|(_, path, _)| assets.load(*path))
                .collect();
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

    let Asked::Waiting(asked) = &faces.asked else {
        unreachable!()
    };
    let ready: Vec<(String, Vec<u8>, As)> = STARTUP
        .iter()
        .zip(asked)
        .filter_map(|((name, _, how), handle)| {
            loaded
                .get(handle)
                .map(|face| ((*name).to_owned(), face.0.clone(), *how))
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
