//! The faces egui is set in, and the one place its font set is built.
//!
//! egui holds a single `FontDefinitions` and `set_fonts` replaces it whole, so two places
//! installing faces means the second silently undoes the first. Everything that adds one goes
//! through [`Faces::install`], which keeps the accumulated set and hands egui all of it.
//!
//! The interface itself is egui's own face. What is here is the two surfaces that are not the
//! interface: the radio, and the reader's page.
//!
//! Fetched through the asset server rather than compiled in, for the reason
//! [`crate::library::FACES`] gives: the browser build takes them from the CDN, and a face in
//! the binary is bytes every player downloads whether or not anything sets type in it.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::library::FontFace;

/// Geo: the radio logs, and nothing else. What a ship said is set apart from the window that
/// is showing it — see [`crate::radio_panel`].
pub const RADIO: &str = "radio";

/// What is asked for at startup, before anything is drawn. The reader asks for its own when a
/// book is first opened.
const STARTUP: &[(&str, &str, As)] = &[(RADIO, "fonts/Geo-Regular.ttf", As::Named)];

/// How a face joins the set.
#[derive(Clone, Copy)]
pub enum As {
    /// A family of its own, with egui's own faces under it, so a glyph this face lacks is
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
        Self { defs: egui::FontDefinitions::default(), asked: Asked::Unasked }
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
        for (name, bytes, how) in added {
            self.defs.font_data.insert(
                name.clone(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            // A fallback list is a list of keys into `font_data`, and epaint answers one that
            // names a font it does not hold by panicking rather than by leaving a glyph out.
            // So it is taken from the set as it stands and never written ahead of a face.
            let mut list = vec![name.clone()];
            if let As::Named = how {
                let under = self.defs.families.get(&egui::FontFamily::Proportional);
                list.extend(under.cloned().unwrap_or_default());
            }
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

/// Asks for the startup faces and installs them together once they have all landed.
///
/// Together, because a surface half in one face and half in another for the frame between them
/// arriving would reflow under whatever is being read. Until then egui draws in its own face,
/// which is what the interface is set in anyway.
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
            let asked = STARTUP.iter().map(|(_, path, _)| assets.load(*path)).collect();
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
    let ready: Vec<(String, Vec<u8>, As)> = STARTUP
        .iter()
        .zip(asked)
        .filter_map(|((name, _, how), handle)| {
            loaded.get(handle).map(|face| ((*name).to_owned(), face.0.clone(), *how))
        })
        .collect();
    let names: Vec<String> = ready.iter().map(|(name, _, _)| name.clone()).collect();
    match names.is_empty() {
        true => info!("no {RADIO} face; the log is set in the interface font"),
        false => info!("settled {}", names.join(", ")),
    }
    faces.install(&ctx, ready);
    faces.asked = Asked::Settled;
}
