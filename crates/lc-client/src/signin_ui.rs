//! The sign-in modal, and the machinery behind it.
//!
//! Desktop only. In a browser the page that launched the game already has a session and hands
//! over a ticket; there is nothing to ask.
//!
//! Every network call and every socket happens on a Bevy IO task and reports back through a
//! channel, because a frame that blocks on a broker is a frame that never renders.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use em_ui::{MenuTheme, MenuUi};

use bevy_egui::{EguiContexts, egui};

use crate::action::Action;
use crate::app::{AppState, Ui};
use crate::auth::{BrokerError, Identity, Session};
use crate::broker::Broker;
use crate::input::Requested;
use crate::ui::MenuPage;
use crate::vault::Vault;

/// Where the broker is, and which server tickets are for.
///
/// From the environment so a development build can point at a local broker without a rebuild,
/// and absent means there is no sign-in at all — which is the state every test and every
/// `--shot` run is in.
#[derive(Resource)]
pub struct Signin {
    pub broker: Option<Broker>,
    pub vault: Vault,
    pub session: Session,
    /// What the client holds between launches, once it has one.
    pub grant: Option<String>,
    /// The password form, when it is open. See [`Form`].
    pub form: Option<Form>,
    /// Behind a lock for the reason `Loopback`'s is: a resource must be `Sync`.
    from_tasks: Mutex<Receiver<Report>>,
    to_main: Sender<Report>,
}

/// What the player has typed.
///
/// egui rather than Bevy UI, unlike the rest of the menu. Doc 13's rule is that anything dense
/// with text is egui and the menu is the exception; two text fields and their labels are dense
/// with text, and Bevy UI has no text input to build them out of.
#[derive(Default)]
pub struct Form {
    pub email: String,
    pub password: String,
    pub display_name: String,
    /// Making an account rather than using one. The reason the password provider exists is
    /// that a development environment needs a hundred of them.
    pub registering: bool,
}

/// What a background task has to say.
enum Report {
    Granted { grant: String, identity: Identity },
    Failed(String),
    /// The grant we had is no good. Distinct from a failure, because only this clears the vault.
    Rejected,
}

impl Signin {
    pub fn new(config_dir: PathBuf) -> Self {
        let broker = match (std::env::var("LC_BROKER"), std::env::var("LC_SHARD")) {
            (Ok(base), Ok(shard)) if !base.is_empty() => Some(Broker::new(&base, &shard)),
            _ => None,
        };
        let (to_main, from_tasks) = channel();
        Self {
            broker,
            vault: Vault::best(config_dir),
            session: Session::default(),
            grant: None,
            form: None,
            from_tasks: Mutex::new(from_tasks),
            to_main,
        }
    }

    /// Whether the player may start observing.
    ///
    /// **A client with no broker configured is ready.** That is the development and offline
    /// case, and gating a single-player sky behind a sign-in nobody asked for would be the
    /// wrong failure.
    pub fn may_observe(&self) -> bool {
        self.broker.is_none() || self.session.is_ready()
    }
}

pub struct SigninPlugin;

impl Plugin for SigninPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Signin::new(config_dir()))
            .add_systems(Startup, resume)
            .add_systems(OnEnter(AppState::MainMenu), open_dev_form)
            .add_systems(
                Update,
                (handle, collect, listen, press)
                    .chain()
                    .in_set(crate::app::Stage::Act)
                    .run_if(in_state(AppState::MainMenu)),
            )
            .add_systems(
                Update,
                draw.in_set(crate::app::Stage::Act)
                    .run_if(in_state(AppState::MainMenu))
                    .after(collect),
            )
            .add_systems(
                bevy_egui::EguiPrimaryContextPass,
                form.run_if(in_state(AppState::MainMenu)),
            );
    }
}

/// Where a device grant is kept when there is no keychain.
///
/// Per-platform, and not the XDG layout applied everywhere: this used to write
/// `~/.config/lightcone` on macOS and Windows too, which is the right answer on one of the
/// three platforms it runs on. `ProjectDirs` gives `~/Library/Application Support/lightcone`,
/// `%APPDATA%\lightcone` and `$XDG_CONFIG_HOME/lightcone` respectively.
///
/// The keychain is still preferred over any of them — see [`crate::vault::Vault::best`]. This
/// is where the fallback lands, and the fallback is a file.
fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "lightcone")
        .map(|dirs| dirs.config_dir().to_path_buf())
        // No home directory to put it in. A grant here does not survive a reboot, which is
        // worse than the keychain and better than signing in every launch.
        .unwrap_or_else(|| std::env::temp_dir().join("lightcone"))
}

/// Trade the stored grant for an identity, at startup.
///
/// The grant is what makes a sixty-second ticket workable: a returning player is signed in
/// before they reach the menu, and never sees this module at all.
fn resume(mut signin: ResMut<Signin>) {
    let Some(broker) = signin.broker.clone() else { return };
    let Ok(Some(grant)) = signin.vault.read() else { return };
    signin.grant = Some(grant.clone());
    signin.session = Session::Working;
    let to_main = signin.to_main.clone();
    IoTaskPool::get()
        .spawn(async move {
            // The ticket is not used to connect — it is asked for to find out whether the
            // grant is still good, and the claims say who it belongs to. See
            // `broker::identity_in` for why reading them unverified is right here.
            let _ = match broker.ticket(&grant) {
                Ok(ticket) => to_main.send(Report::Granted {
                    grant: grant.clone(),
                    identity: crate::broker::identity_in(&ticket).unwrap_or(Identity {
                        account_id: String::new(),
                        display_name: "signed in".into(),
                    }),
                }),
                Err(BrokerError::Refused) => to_main.send(Report::Rejected),
                Err(why) => to_main.send(Report::Failed(why.to_string())),
            };
        })
        .detach();
}

/// Act on the sign-in actions.
///
/// Read from the same `Requested` stream everything else is, rather than through the action
/// fold: these need a socket, a browser and the vault, none of which belong in a pure fold.
fn handle(
    mut requests: MessageReader<Requested>,
    mut signin: ResMut<Signin>,
    mut ui: ResMut<Ui>,
) {
    for request in requests.read() {
        match &request.0 {
            Action::SignIn => begin(&mut signin),
            Action::CancelSignIn => {
                // Dropping the listener closes the socket, so the browser tab that never came
                // back gets a refused connection rather than hanging.
                signin.session = Session::SignedOut;
                ui.menu_page = MenuPage::Root;
            }
            Action::SignOut => {
                let _ = signin.vault.clear();
                signin.grant = None;
                signin.session = Session::SignedOut;
            }
            // Opening the form is all this does; submitting it is `password_submit`, which
            // the egui panel calls directly because it owns the typed text.
            Action::SignInWithPassword { .. } => signin.form = Some(Form::default()),
            _ => {}
        }
    }
}

/// Open the form `--password` asked for, so it can be photographed.
fn open_dev_form(dev: Res<crate::dev::DevEntry>, mut signin: ResMut<Signin>) {
    if dev.open_password_form {
        signin.form = Some(Form::default());
    }
}

/// Take what the background tasks have said.
fn collect(mut signin: ResMut<Signin>) {
    let reports: Vec<Report> = {
        let Ok(from_tasks) = signin.from_tasks.lock() else { return };
        from_tasks.try_iter().collect()
    };
    for report in reports {
        match report {
            Report::Granted { grant, identity } => {
                if let Err(why) = signin.vault.write(&grant) {
                    warn!("could not keep the sign-in: {why}");
                }
                signin.grant = Some(grant);
                signin.session = Session::SignedIn(identity);
            }
            // Only a refusal clears the vault. A broker that is down must not sign everyone
            // out, because none of them could sign back in.
            Report::Rejected => {
                let _ = signin.vault.clear();
                signin.grant = None;
                signin.session = Session::SignedOut;
            }
            Report::Failed(why) => signin.session = Session::Failed(why),
        }
    }
}

/// Watch the loopback for the browser coming back.
fn listen(mut signin: ResMut<Signin>) {
    let Session::Waiting { loopback, .. } = &signin.session else { return };
    let Some(answer) = loopback.poll() else { return };
    let Some(broker) = signin.broker.clone() else { return };

    let code = match answer {
        Ok(code) => code,
        Err(why) => {
            signin.session = Session::Failed(format!("that sign-in did not come back right: {why:?}"));
            return;
        }
    };
    let Session::Waiting { loopback, .. } = std::mem::replace(&mut signin.session, Session::Working)
    else {
        return;
    };
    let return_to = format!("http://127.0.0.1:{}{}", loopback.port, crate::auth::RETURN_PATH);
    let label = machine_name();
    let to_main = signin.to_main.clone();
    IoTaskPool::get()
        .spawn(async move {
            let _ = match broker.redeem(&code, &return_to, &label) {
                Ok(granted) => to_main
                    .send(Report::Granted { grant: granted.grant, identity: granted.identity }),
                Err(why) => to_main.send(Report::Failed(why.to_string())),
            };
        })
        .detach();
}

fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// What a revocation list shows a person later, so it is the machine and not an identifier.
fn machine_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "a desktop".into())
}

/// Begin: open the browser, listen on loopback.
pub fn begin(signin: &mut Signin) {
    let Some(broker) = signin.broker.clone() else { return };
    let bound = match crate::auth::Bound::open() {
        Ok(bound) => bound,
        Err(why) => {
            signin.session = Session::Failed(format!("could not listen for the answer: {why}"));
            return;
        }
    };
    // A nonce, from the world's own entropy rather than a crate: this has to be unguessable
    // and it has to be different every time, which is all `rng::hash` promises and all it needs
    // to promise, since the value never leaves this machine except in a URL the player opens.
    let state = format!(
        "{:016x}{:016x}",
        lc_world::rng::hash(&[now_nanos(), std::process::id() as u64]),
        lc_world::rng::hash(&[now_nanos().wrapping_mul(0x9e37_79b9), bound.port as u64]),
    );

    let pending = crate::auth::begin(&broker.base, bound.port, &state);
    let url = pending.open.clone();
    // Best effort. A browser that will not open leaves the address on screen to be copied,
    // which is why it is kept rather than only logged.
    if let Err(why) = webbrowser::open(&url) {
        warn!("could not open a browser ({why}); the address is on screen");
    }
    signin.session = Session::Waiting { loopback: bound.listen(pending), url };
}

/// Send what was typed.
///
/// Doc 16 records why this form exists and why it is an argument against the password provider
/// reaching production: it teaches a player to type a credential into a game window.
fn password_submit(signin: &mut Signin) {
    let Some(broker) = signin.broker.clone() else { return };
    let Some(form) = signin.form.take() else { return };
    let register_as = form.registering.then(|| {
        let typed = form.display_name.trim();
        if typed.is_empty() { "Traveler".to_string() } else { typed.to_string() }
    });
    let label = machine_name();
    let to_main = signin.to_main.clone();
    signin.session = Session::Working;
    IoTaskPool::get()
        .spawn(async move {
            let asked =
                broker.with_password(&form.email, &form.password, &label, register_as.as_deref());
            let _ = match asked {
                Ok(granted) => to_main
                    .send(Report::Granted { grant: granted.grant, identity: granted.identity }),
                Err(why) => to_main.send(Report::Failed(why.to_string())),
            };
        })
        .detach();
}

/// The password form, which is the one egui surface in the menu.
fn form(mut contexts: EguiContexts, mut signin: ResMut<Signin>, ui_state: Res<Ui>) {
    if ui_state.menu_page != MenuPage::SignIn || signin.form.is_none() {
        return;
    }
    let Ok(context) = contexts.ctx_mut() else { return };

    let mut submit = false;
    let mut cancel = false;
    // Dressed in the menu's palette. The game's other egui panels are default dark and that is
    // fine where they sit, over a rendered sky; this one is inside the menu, and a gray box in
    // the middle of it reads as a different application rather than as part of this one.
    egui::Window::new("Password")
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .frame(egui::Frame {
            // Opaque. The Bevy modal is behind it and a 92% panel lets its buttons read
            // straight through the form, which is the same muddle the modal itself had.
            fill: color(em_ui::vfd::PANEL_BG.with_alpha(1.0_f32)),
            stroke: egui::Stroke::new(1.0_f32, color(em_ui::vfd::BUTTON_BORDER)),
            inner_margin: egui::Margin::same(14),
            ..egui::Frame::NONE
        })
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(context, |ui| {
            let text = color(em_ui::vfd::TEXT);
            ui.visuals_mut().override_text_color = Some(text);
            ui.visuals_mut().widgets.inactive.bg_fill = color(em_ui::vfd::BUTTON_BG);
            ui.visuals_mut().widgets.hovered.bg_fill = color(em_ui::vfd::BUTTON_HOVER);
            ui.visuals_mut().widgets.active.bg_fill = color(em_ui::vfd::BUTTON_HOVER);
            // What a text field is filled with. Darker than the panel, or a field with nothing
            // in it is invisible and the form looks like labels with no inputs.
            ui.visuals_mut().extreme_bg_color = egui::Color32::from_rgb(4, 14, 8);
            ui.visuals_mut().widgets.inactive.bg_stroke =
                egui::Stroke::new(1.0_f32, color(em_ui::vfd::TEXT_DIM));
            let Some(form) = signin.form.as_mut() else { return };
            ui.set_min_width(320.0);
            ui.heading("Password");
            ui.add_space(6.0);
            egui::Grid::new("credentials").num_columns(2).show(ui, |ui| {
                ui.label("Email");
                ui.text_edit_singleline(&mut form.email);
                ui.end_row();
                ui.label("Password");
                // `password(true)` is not decoration: this is on screen in a game, which is
                // more likely to be streamed or screenshotted than a browser is.
                ui.add(egui::TextEdit::singleline(&mut form.password).password(true));
                ui.end_row();
                if form.registering {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut form.display_name);
                    ui.end_row();
                }
            });
            ui.checkbox(&mut form.registering, "Create an account");
            ui.separator();
            ui.horizontal(|ui| {
                let ready = !form.email.trim().is_empty() && !form.password.is_empty();
                submit = ui.add_enabled(ready, egui::Button::new("Sign in")).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });

    if submit {
        password_submit(&mut signin);
    } else if cancel {
        signin.form = None;
    }
}

/// A Bevy color as an egui one, so the two surfaces share a palette rather than a guess.
fn color(from: bevy::prelude::Color) -> egui::Color32 {
    let rgba = from.to_srgba();
    egui::Color32::from_rgba_unmultiplied(
        (rgba.red * 255.0) as u8,
        (rgba.green * 255.0) as u8,
        (rgba.blue * 255.0) as u8,
        (rgba.alpha * 255.0) as u8,
    )
}

/// What the modal is showing.
///
/// Compared against what is already drawn, so the modal is rebuilt when its *contents* change
/// and not when anything else does. Without this it was rebuilt every frame — the menu's
/// backdrop drift writes `Ui` each frame, `Ui::is_changed` is therefore always true, and a
/// button that is despawned and respawned before the next frame can never be hovered.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Shown {
    /// The form owns the screen; the modal is a backdrop and nothing else.
    Backdrop,
    Choice,
    Waiting(String),
    Working,
    Failed(String),
    SignedIn(String),
}

impl Shown {
    fn of(session: &Session, form_open: bool) -> Self {
        if form_open {
            return Shown::Backdrop;
        }
        match session {
            Session::SignedOut => Shown::Choice,
            Session::Waiting { url, .. } => Shown::Waiting(url.clone()),
            Session::Working => Shown::Working,
            Session::Failed(why) => Shown::Failed(why.clone()),
            Session::SignedIn(identity) => Shown::SignedIn(identity.display_name.clone()),
        }
    }
}

/// The modal.
fn draw(
    mut commands: Commands,
    ui: Res<Ui>,
    signin: Res<Signin>,
    assets: Res<AssetServer>,
    drawn: Query<(Entity, &Modal)>,
) {
    let wanted = (ui.menu_page == MenuPage::SignIn)
        .then(|| Shown::of(&signin.session, signin.form.is_some()));

    // Already showing the right thing: leave it alone. Every rebuild resets the hover state of
    // every button in it.
    if let Ok((entity, Modal(showing))) = drawn.single() {
        if Some(showing) == wanted.as_ref() {
            return;
        }
        commands.entity(entity).despawn();
    }
    let Some(wanted) = wanted else { return };

    // Opaque, unlike an ordinary panel. The menu is still there behind it, dimmed, and two
    // translucent panels of the same size at the same place read as one muddled thing rather
    // than as one in front of the other.
    let theme = MenuTheme { panel_bg: MenuTheme::VFD.panel_bg.with_alpha(1.0), ..MenuTheme::VFD };
    // No wordmark: the heading here is "SIGN IN", not the game's name, so the title falls
    // back to the interface face like every other line on the screen.
    let mut menu = MenuUi::new(&mut commands, theme)
        .panel_width(460.0)
        .font(assets.load(crate::faces::UI_FILE));
    let screen = menu.overlay(Modal(wanted.clone()));
    // The form owns the screen while it is up, for the reason the menu stands down for this
    // modal: one surface at a time. The backdrop stays, so the sky is still dimmed behind it.
    let Shown::Backdrop = &wanted else {
        let panel = menu.panel(screen);
        menu.title(panel, "SIGN IN");
        match &wanted {
            Shown::SignedIn(name) => {
                menu.message(panel, &format!("Signed in as {name}."));
                menu.button(panel, "Observe", Emit(Action::StartGame));
                menu.button(panel, "Sign out", Emit(Action::SignOut));
            }
            Shown::Waiting(url) => {
                menu.message(panel, "Finish signing in with your browser.");
                // The address itself, because a browser that did not open leaves a player with
                // nothing to act on otherwise.
                menu.message(panel, url);
                menu.button(panel, "Cancel", Emit(Action::CancelSignIn));
            }
            Shown::Working => {
                menu.message(panel, "One moment…");
            }
            Shown::Failed(why) => {
                menu.message(panel, why);
                menu.button(panel, "Try again", Emit(Action::SignIn));
                menu.button(panel, "Back", Emit(Action::GoToMenuPage(MenuPage::Root)));
            }
            Shown::Choice => {
                menu.message(panel, "Observing needs an account.");
                menu.button(panel, "Sign in with a browser", Emit(Action::SignIn));
                // The local one. Offered always and refused with a reason by a deployment that
                // has no password provider, rather than the client asking what is on offer.
                menu.button(
                    panel,
                    "Use a password",
                    Emit(Action::SignInWithPassword {
                        email: String::new(),
                        password: String::new(),
                    }),
                );
                menu.button(panel, "Back", Emit(Action::GoToMenuPage(MenuPage::Root)));
            }
            Shown::Backdrop => unreachable!("handled above"),
        }
        return;
    };
}

#[derive(Component)]
struct Modal(Shown);

/// What a button asks for. The menu's own `Emit`, which is private to it.
#[derive(Component)]
pub struct Emit(pub Action);

pub fn press(buttons: Query<(&Interaction, &Emit), Changed<Interaction>>, mut out: MessageWriter<Requested>) {
    for (interaction, emit) in &buttons {
        if *interaction == Interaction::Pressed {
            out.write(Requested(emit.0.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client with no broker configured may observe. Gating an offline sky behind a sign-in
    /// nobody asked for would be the wrong failure, and it is the state every `--shot` run and
    /// every test is in.
    #[test]
    fn no_broker_means_no_sign_in_is_required() {
        let signin = Signin {
            broker: None,
            vault: Vault::memory(),
            session: Session::SignedOut,
            grant: None,
            form: None,
            from_tasks: Mutex::new(channel().1),
            to_main: channel().0,
        };
        assert!(signin.may_observe());
    }

    /// With one configured, an identity is required — and only an identity will do.
    #[test]
    fn a_configured_broker_means_signing_in_first() {
        let with = |session| Signin {
            broker: Some(Broker::new("https://accounts.example", "shard-1")),
            vault: Vault::memory(),
            session,
            grant: None,
            form: None,
            from_tasks: Mutex::new(channel().1),
            to_main: channel().0,
        };
        assert!(!with(Session::SignedOut).may_observe());
        assert!(!with(Session::Working).may_observe());
        assert!(!with(Session::Failed("no".into())).may_observe());
        assert!(
            with(Session::SignedIn(Identity {
                account_id: "a".into(),
                display_name: "Ada".into(),
            }))
            .may_observe()
        );
    }

    /// The bug this exists to prevent: a modal rebuilt every frame has buttons that are
    /// destroyed and respawned before a hover can register, so they never light up.
    ///
    /// The menu's backdrop drift writes `Ui` every frame, so `Ui::is_changed` is *always* true
    /// and cannot be the trigger. What is drawn has to be compared instead.
    #[test]
    fn the_modal_is_rebuilt_only_when_its_contents_change() {
        let signed_out = Shown::of(&Session::SignedOut, false);
        assert_eq!(signed_out, Shown::of(&Session::SignedOut, false), "the same state differs");

        // Opening the form changes it, and so does every step of signing in.
        assert_ne!(signed_out, Shown::of(&Session::SignedOut, true));
        assert_ne!(signed_out, Shown::of(&Session::Working, false));
        assert_ne!(
            Shown::of(&Session::Failed("one".into()), false),
            Shown::of(&Session::Failed("another".into()), false),
            "a different message must redraw",
        );
        assert_ne!(
            Shown::of(
                &Session::SignedIn(Identity { account_id: "a".into(), display_name: "Ada".into() }),
                false,
            ),
            Shown::of(
                &Session::SignedIn(Identity { account_id: "a".into(), display_name: "Grace".into() }),
                false,
            ),
        );
    }

    /// While the form is up the modal is a backdrop and nothing else, whatever is behind it in
    /// the session — so it does not redraw as the sign-in progresses underneath.
    #[test]
    fn the_form_owns_the_screen_whatever_the_session_says() {
        for session in [Session::SignedOut, Session::Working, Session::Failed("x".into())] {
            assert_eq!(Shown::of(&session, true), Shown::Backdrop);
        }
    }

    /// The config directory is somewhere the player owns, named for this game, and **where
    /// this platform keeps such things** — not the XDG layout applied to all three.
    #[test]
    fn the_grant_is_kept_where_this_platform_keeps_things() {
        let dir = config_dir();
        assert!(dir.ends_with("lightcone"), "{dir:?}");
        let said = dir.to_string_lossy();

        #[cfg(target_os = "macos")]
        assert!(
            said.contains("Library/Application Support"),
            "macOS does not keep this in {said}",
        );
        #[cfg(target_os = "windows")]
        assert!(said.contains("AppData"), "Windows does not keep this in {said}");
        #[cfg(target_os = "linux")]
        assert!(
            said.contains(".config") || said.contains("XDG"),
            "Linux does not keep this in {said}",
        );
    }
}
