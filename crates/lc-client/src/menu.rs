//! The main menu: Bevy UI in front of a drifting sky.
//!
//! Bevy UI rather than egui for this one screen, because it composites over a rendered
//! background; everything dense with text stays in egui. The buttons emit actions like every
//! other surface here, so the menu reads state and changes none.
//!
//! The sky behind it is scenery, not the world. The catalogue is a hundred and twenty thousand
//! rows and loading it is what `AppState::Loading` exists for, so the menu generates its own
//! field of stars instead - drawn through the real starfield pass, at rest, so it is the same
//! sky the game draws rather than a picture of one.

use bevy::prelude::*;
use em_ui::{MenuTheme, MenuUi};
use glam::DVec3;
use lc_world::rng;
use lc_world::sky::{AuthoredStars, CatalogueStar, Component as StarComponent, Provenance, StarId};
use lc_world::star::Star;

use crate::action::Action;
use crate::app::{AppState, Game, Ui};
use crate::input::Requested;
use crate::session::Session;
use crate::ui::MenuPage;

pub struct MainMenuPlugin;

impl Plugin for MainMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(em_ui::MenuUiPlugin)
            .add_systems(
                OnEnter(AppState::MainMenu),
                (install_backdrop, open_dev_page, crate::starfield::spawn_sky).chain(),
            )
            .add_systems(OnExit(AppState::MainMenu), cleanup)
            .add_systems(
                Update,
                (sync_screen, drift, press)
                    .chain()
                    .in_set(crate::app::Stage::Act)
                    .run_if(in_state(AppState::MainMenu)),
            );
    }
}

/// Nabla, and the title screen only: the game's name as a wordmark, the same face the site
/// sets it in. A display face on a button or a readout would be illegible and costs a megabyte
/// and a half, so it is loaded here, on the one screen that draws it.
const WORDMARK: &str = "fonts/Nabla.ttf";
/// What that face wants. Nabla is an extruded three-dimensional design and its depth is inside
/// the glyph, so at the size a heading is set at there is nothing left to read.
const WORDMARK_SIZE: f32 = 44.0;

/// The page currently on screen, so a change of [`MenuPage`] can be noticed without a second
/// copy of it: the interface state stays the only place the page is recorded.
#[derive(Component)]
struct MenuScreen(MenuPage);

/// What a button asks for when it is pressed.
#[derive(Component)]
struct Emit(Action);

fn sync_screen(
    mut commands: Commands,
    ui: Res<Ui>,
    assets: Res<AssetServer>,
    existing: Query<(Entity, &MenuScreen)>,
    observe: ObserveEmits,
) {
    let page = ui.menu_page;
    if let Ok((entity, drawn)) = existing.single() {
        if drawn.0 == page {
            return;
        }
        commands.entity(entity).despawn();
    }
    build(&mut commands, page, observe_action(&observe), assets.load(WORDMARK));
}

/// What the Observe button asks for.
///
/// Signing in is a condition of observing, not a separate menu item: a player who is signed in
/// never sees the modal, and a player who is not is taken to it by the button they already
/// meant to press.
#[cfg(not(target_arch = "wasm32"))]
type ObserveEmits<'w> = Option<Res<'w, crate::signin_ui::Signin>>;
#[cfg(target_arch = "wasm32")]
type ObserveEmits<'w> = std::marker::PhantomData<&'w ()>;

#[cfg(not(target_arch = "wasm32"))]
fn observe_action(signin: &ObserveEmits) -> Action {
    match signin {
        Some(signin) if !signin.may_observe() => Action::GoToMenuPage(MenuPage::SignIn),
        _ => Action::StartGame,
    }
}

/// The browser build arrives with a session already, so there is nothing to ask.
#[cfg(target_arch = "wasm32")]
fn observe_action(_: &ObserveEmits) -> Action {
    Action::StartGame
}

fn build(commands: &mut Commands, page: MenuPage, observe: Action, wordmark: Handle<Font>) {
    let mut ui = MenuUi::new(commands, MenuTheme::VFD)
        .panel_width(520.0)
        .title_font(wordmark, WORDMARK_SIZE);
    let root = ui.screen(MenuScreen(page));
    // **One surface at a time.** The sign-in draws its own, and a menu behind it is a second
    // thing to read and a second set of buttons to try. The screen is still spawned, because
    // it carries the marker that says which page is drawn.
    if page == MenuPage::SignIn {
        return;
    }
    let panel = ui.panel(root);
    ui.title(panel, "LIGHTCONE FRONTIER");
    ui.message(panel, "Everything you see has already happened.");

    match page {
        MenuPage::Root => {
            ui.button(panel, "Observe", Emit(observe));
            ui.button(panel, "Settings", Emit(Action::GoToMenuPage(MenuPage::Settings)));
            ui.button(panel, "Quit", Emit(Action::Quit));
        }
        // Returned above; the sign-in owns the screen while it is up.
        MenuPage::SignIn => {}
        other => {
            ui.message(panel, &format!("{other:?}"));
            ui.button(panel, "Back", Emit(Action::GoToMenuPage(MenuPage::Root)));
        }
    }
}

fn press(buttons: Query<(&Interaction, &Emit), Changed<Interaction>>, mut out: MessageWriter<Requested>) {
    for (interaction, emit) in &buttons {
        if *interaction == Interaction::Pressed {
            out.write(Requested(emit.0.clone()));
        }
    }
}

/// One radian a minute about the pole, which is what Exotic Matters' menu drifts at.
///
/// Yaw only: [`crate::ui::Look`] carries no roll, and a horizon that turns is a worse backdrop
/// than one that pans. The heading the drift reaches is where the ship starts looking, which
/// is as good a heading as any.
const DRIFT_RATE: f64 = 1.0 / 60.0;

/// Pitch the field sits at, so the drift pans across it rather than around the pole.
const DRIFT_PITCH: f64 = 0.12;

/// Open the page `--signin` asked for, so it can be photographed.
fn open_dev_page(dev: Res<crate::app::DevEntry>, mut ui: ResMut<Ui>) {
    if let Some(page) = dev.menu_page {
        ui.menu_page = page;
    }
}

fn drift(time: Res<Time>, mut ui: ResMut<Ui>) {
    ui.look.pitch = DRIFT_PITCH;
    ui.look.turn(DRIFT_RATE * time.delta_secs_f64(), 0.0);
}

fn cleanup(mut commands: Commands, screens: Query<Entity, With<MenuScreen>>) {
    for entity in &screens {
        commands.entity(entity).despawn();
    }
}

/// Put the backdrop sky where the starfield pass looks for a sky.
///
/// [`crate::app::load_world`] replaces this wholesale when a world is loaded, so the menu's
/// session is never the one played.
fn install_backdrop(mut game: ResMut<Game>) {
    game.0 = Session::new(&backdrop_sky(BACKDROP_STARS, BACKDROP_SEED), BACKDROP_STARS);
}

const BACKDROP_STARS: usize = 12000;

/// Fixed, so the menu looks the same every launch. A backdrop that is different each time
/// reads as a bug in the sky rather than as variety.
const BACKDROP_SEED: u64 = 0x11_6b_7c_0e;

/// How far out the backdrop reaches, light-years. Far enough that a Salpeter tail puts a
/// handful of B stars in the field; near enough that the M dwarfs which are most of it are
/// still above the floor.
const BACKDROP_RADIUS_LY: f64 = 400.0;

/// Inside this the starfield would draw a star as a nearby object rather than as background -
/// see [`crate::starfield::LOCAL_SHELL_LY`]. No backdrop star may be there.
const BACKDROP_MIN_LY: f64 = 8.0;

/// A field of main-sequence stars, isotropic and uniform in volume.
fn backdrop_sky(count: usize, seed: u64) -> AuthoredStars {
    let stars = (0..count as u64).map(|i| backdrop_star(seed, i)).collect();
    AuthoredStars::new("menu backdrop", stars)
}

fn backdrop_star(seed: u64, index: u64) -> CatalogueStar {
    let draw = |salt: u64| rng::uniform(rng::hash(&[seed, index, salt]));

    // Uniform on the sphere, and uniform in volume along the radius.
    let z = draw(1) * 2.0 - 1.0;
    let phi = draw(2) * std::f64::consts::TAU;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let distance = (BACKDROP_MIN_LY.powi(3)
        + draw(3) * (BACKDROP_RADIUS_LY.powi(3) - BACKDROP_MIN_LY.powi(3)))
    .cbrt();
    let position_ly = DVec3::new(r * phi.cos(), r * phi.sin(), z) * distance;

    let mass = salpeter_mass(draw(4));
    // Main-sequence mass-luminosity, and an effective temperature consistent with it. Rough,
    // but the backdrop is judged by eye and the two have to agree or the colours will not
    // match the sizes.
    let luminosity_solar = mass.powf(3.5);
    let teff_k = em_spectra::stellar::SOLAR_TEFF * mass.powf(0.505);
    let luminosity_w = luminosity_solar * em_spectra::stellar::SOLAR_LUMINOSITY;

    CatalogueStar {
        id: StarId::synthesise("menu", index),
        provenance: Provenance { source: "menu".into(), key: index },
        name: None,
        position_ly,
        velocity: DVec3::ZERO,
        star: Star {
            radius_m: em_spectra::stellar::radius_from_luminosity(luminosity_w, teff_k),
            teff_k,
            mu: em_spectra::stellar::mu_from_mass_solar(mass),
            limb_darkening: (0.4, 0.26),
        },
        luminosity_solar,
        mass_solar: mass,
        metallicity: 0.0,
        component: StarComponent { index: 1, group: None },
    }
}

/// A mass in solar units from a uniform draw, by inverting the Salpeter initial mass function.
///
/// `p(m) ~ m^-2.35`, so the cumulative distribution inverts in closed form. The cut-off at
/// eighteen solar masses is what keeps a field of four thousand from containing an O star that
/// would drown the rest of it.
fn salpeter_mass(u: f64) -> f64 {
    const LOW: f64 = 0.15;
    const HIGH: f64 = 18.0;
    const EXPONENT: f64 = -1.35;
    let (lo, hi) = (LOW.powf(EXPONENT), HIGH.powf(EXPONENT));
    (lo + u * (hi - lo)).powf(1.0 / EXPONENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_backdrop_star_is_inside_the_local_shell() {
        let sky = backdrop_sky(512, BACKDROP_SEED);
        let nearest = <AuthoredStars as lc_world::sky::StarProvider>::stars(&sky)
            .iter()
            .map(|s| s.position_ly.length())
            .fold(f64::INFINITY, f64::min);
        assert!(
            nearest > crate::starfield::LOCAL_SHELL_LY,
            "a backdrop star at {nearest} ly would be drawn as a local object"
        );
    }

    /// The field has to be mostly dwarfs, or the sky is a wall of blue giants.
    #[test]
    fn the_backdrop_follows_an_initial_mass_function() {
        let sky = backdrop_sky(2000, BACKDROP_SEED);
        let stars = <AuthoredStars as lc_world::sky::StarProvider>::stars(&sky);
        let dwarfs = stars.iter().filter(|s| s.mass_solar < 1.0).count();
        let giants = stars.iter().filter(|s| s.mass_solar > 8.0).count();
        assert!(dwarfs * 10 > stars.len() * 8, "only {dwarfs} of {} are dwarfs", stars.len());
        assert!(giants < stars.len() / 100, "{giants} stars above eight solar masses");
    }
}
