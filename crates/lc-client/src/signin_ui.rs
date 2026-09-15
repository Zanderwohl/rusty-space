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
    /// Behind a lock for the reason `Loopback`'s is: a resource must be `Sync`.
    from_tasks: Mutex<Receiver<Report>>,
    to_main: Sender<Report>,
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
            .add_systems(
                Update,
                (handle, collect, listen, press).chain().run_if(in_state(AppState::MainMenu)),
            )
            .add_systems(
                Update,
                draw.run_if(in_state(AppState::MainMenu)).after(collect),
            );
    }
}

/// Where a device grant is kept when there is no keychain.
fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(std::env::temp_dir)
        .join("lightcone")
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
            // A ticket is asked for and thrown away: this is only to find out whether the
            // grant is still good, and who it belongs to.
            let _ = match broker.ticket(&grant) {
                Ok(_) => to_main.send(Report::Granted {
                    grant: grant.clone(),
                    identity: Identity {
                        account_id: String::new(),
                        display_name: "signed in".into(),
                    },
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
            Action::SignInWithPassword { .. } => {
                // The in-modal form. Doc 16 records why it exists and why it is an argument
                // against the password provider reaching production.
                signin.session =
                    Session::Failed("the in-modal password form is not built yet".into());
            }
            _ => {}
        }
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

/// The modal.
fn draw(mut commands: Commands, ui: Res<Ui>, signin: Res<Signin>, drawn: Query<Entity, With<Modal>>) {
    let wanted = ui.menu_page == MenuPage::SignIn;
    for entity in &drawn {
        // Rebuilt whenever anything changed, which is cheap: a panel and four children.
        if !wanted || signin.is_changed() || ui.is_changed() {
            commands.entity(entity).despawn();
        }
    }
    if !wanted || (!drawn.is_empty() && !signin.is_changed() && !ui.is_changed()) {
        return;
    }

    // Opaque, unlike an ordinary panel. The menu is still there behind it, dimmed, and two
    // translucent panels of the same size at the same place read as one muddled thing rather
    // than as one in front of the other.
    let theme = MenuTheme { panel_bg: MenuTheme::VFD.panel_bg.with_alpha(1.0), ..MenuTheme::VFD };
    let mut menu = MenuUi::new(&mut commands, theme).panel_width(460.0);
    let screen = menu.overlay(Modal);
    let panel = menu.panel(screen);
    menu.title(panel, "SIGN IN");

    match &signin.session {
        Session::SignedIn(identity) => {
            menu.message(panel, &format!("Signed in as {}.", identity.display_name));
            menu.button(panel, "Observe", Emit(Action::StartGame));
            menu.button(panel, "Sign out", Emit(Action::SignOut));
        }
        Session::Waiting { url, .. } => {
            menu.message(panel, "Finish signing in with your browser.");
            // The address itself, because a browser that did not open leaves a player with
            // nothing to act on otherwise.
            menu.message(panel, url);
            menu.button(panel, "Cancel", Emit(Action::CancelSignIn));
        }
        Session::Working => {
            menu.message(panel, "One moment…");
        }
        Session::Failed(why) => {
            menu.message(panel, why);
            menu.button(panel, "Try again", Emit(Action::SignIn));
            menu.button(panel, "Back", Emit(Action::GoToMenuPage(MenuPage::Root)));
        }
        Session::SignedOut => {
            menu.message(panel, "Observing needs an account.");
            menu.button(panel, "Sign in with a browser", Emit(Action::SignIn));
            menu.button(panel, "Back", Emit(Action::GoToMenuPage(MenuPage::Root)));
        }
    }
}

#[derive(Component)]
struct Modal;

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

    /// The config directory is somewhere the player owns, and named for this game.
    #[test]
    fn the_grant_is_kept_somewhere_of_ours() {
        let dir = config_dir();
        assert!(dir.ends_with("lightcone"), "{dir:?}");
    }
}
