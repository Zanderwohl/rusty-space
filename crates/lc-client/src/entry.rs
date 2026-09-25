//! Turning a request to start the client into a [`DevEntry`].
//!
//! Shared by the desktop binary, which gets its request from `argv`, and the browser one,
//! which gets it from the query string. The two differ only in where the strings come from,
//! and a second copy of this parsing would be a second set of flag names.

use crate::action::Action;
use lc_world::scenario;
use crate::dev::DevEntry;

/// Where this build's assets are.
///
/// Bevy's default resolves `assets` against `CARGO_MANIFEST_DIR` when cargo set it and against
/// the executable's directory otherwise, so a packaged build looked for `target/debug/assets`
/// and found nothing. Checking beside the executable first keeps that working; the compile-time
/// path is the development fallback.
///
/// Here rather than in the desktop binary because the server in the box wants it too: a local
/// shard lends the books sitting next to the client that started it.
#[cfg(not(target_arch = "wasm32"))]
pub fn asset_root() -> std::path::PathBuf {
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("assets")))
        .filter(|path| path.is_dir());
    beside_exe.unwrap_or_else(|| {
        std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"))
    })
}

/// What a request to start the client asked for.
pub struct Entry {
    pub dev: DevEntry,
    /// The catalog to load: the first positional argument, and an **asset** path rather than
    /// a filesystem one. Desktop only; the browser build loads its build's own sky.
    pub catalog: Option<String>,
    /// The shard to connect to. `None` is the single-process game, which is every build before
    /// there was a server to connect to and is still what `--shot` and the snapshot use.
    pub server: Option<String>,
    /// Run a shard in this process and connect to that. Beats `server` when both are given,
    /// because asking for a local one is the more specific request.
    pub local: bool,
    /// A scene to stage, by name. Implies `local`: a scene needs a shard to run in.
    pub demo: Option<String>,
}

/// A mode of the main view by `--view`'s spelling.
fn view_named(name: &str) -> Option<crate::ui::ViewMode> {
    use crate::ui::ViewMode;
    match name.to_ascii_lowercase().as_str() {
        "world" => Some(ViewMode::World),
        "map" => Some(ViewMode::Map),
        "form" | "editor" => Some(ViewMode::Form),
        _ => None,
    }
}

/// Parses the flag vocabulary both binaries accept.
pub fn parse(args: &[String]) -> Entry {
    let flag = |name: &str| args.iter().any(|a| a == name);
    let after = |name: &str| {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    };
    fn value<T: std::str::FromStr>(args: &[String], name: &str) -> Option<T> {
        let i = args.iter().position(|a| a == name)?;
        args.get(i + 1)?.parse().ok()
    }

    let mut actions = Vec::new();
    // `--panel map` is the map's older spelling, from when it was a window. First among the
    // actions as well as pinned (see `DevEntry::view`), so the arrival's `--zoom` and `--turn`
    // reach the camera of the mode they are meant for.
    let view = after("--view")
        .or_else(|| after("--panel").filter(|name| name.eq_ignore_ascii_case("map")))
        .and_then(|name| view_named(&name));
    let editing = view == Some(crate::ui::ViewMode::Form);
    if let Some(view) = view {
        actions.push(Action::SetView(view));
    }
    // A scene says where to stand, so there is nothing to pass in. The identifiers are the
    // scene's own, which is why this needs no shard to have answered first.
    if let Some(scene) = after("--demo").as_deref().and_then(scenario::Scenario::named) {
        if let Some(watch) = crate::action::watching(scene) {
            let crate::ui::CameraPerspective::Pov(ship_id) = watch;
            actions.push(Action::WatchFrom(Some(ship_id)));
        }
    }
    if let Some(preset) = value::<usize>(args, "--band") {
        actions.push(Action::SetBandPreset(preset));
    }
    if let Some(rate) = value::<f64>(args, "--rate") {
        actions.push(Action::SetTimeRate(rate));
    }
    // A book by the name of its file on the shelf, so a page can be photographed.
    if let Some(book) = after("--book") {
        actions.push(Action::OpenBook(book));
    }
    // Which spine document to open at, so a photograph can be of prose rather than of a cover.
    if let Some(chapter) = value::<usize>(args, "--chapter") {
        actions.push(Action::GoTo(chapter, 0));
    }
    // Pages in from wherever the book opened: the only way to photograph a turned page.
    if let Some(pages) = value::<i32>(args, "--pages") {
        actions.push(Action::TurnPage(pages));
    }
    if flag("--beauty") {
        actions.push(Action::SetBeautyShots(true));
    }
    if flag("--tune") {
        actions.push(Action::OpenPanel(crate::ui::Panel::Tuning));
    }
    if let Some(name) = after("--panel")
        && let Some(panel) = crate::ui::Panel::named(&name)
    {
        actions.push(Action::OpenPanel(panel));
    }
    if flag("--watch") || flag("--swarm") {
        actions.push(Action::SelectNearest);
        actions.push(Action::OpenPanel(crate::ui::Panel::Telescope));
    }
    // Telescope duties, so a beauty shot of each can be photographed.
    if flag("--stare") {
        actions.push(Action::SelectNearest);
        actions.push(Action::StareSelected);
    }
    if flag("--sweep") {
        actions.push(Action::SurveyAhead);
    }
    if let Some(b) = value::<usize>(args, "--curve")
        && let Some(band) = em_spectra::Band::ALL.get(b)
    {
        actions.push(Action::SetCurveBand(*band));
    }
    if flag("--fly") {
        // Index 0 of the sorted sky is the Sun in the full catalog; 1 is interstellar.
        actions.push(Action::FlyToNearest);
    }
    // What is selected, as a click on either view would leave it. Everything else that aims is
    // a camera, so this is the only way to photograph a reticle.
    if let Some(name) = after("--focus") {
        let target = match name.strip_prefix("band:").and_then(|n| n.parse::<usize>().ok()) {
            Some(index) => crate::navigation::Target::Band(index),
            None => crate::navigation::Target::Body(name),
        };
        actions.push(Action::FocusTarget(Some(target)));
    }


    // Last, and after anything that aims: `--turn` exists to put something off screen, and
    // `--fly` ends by pointing the view at what it is flying to. Pushed first, the aim undid
    // the turn and the two flags together were the same picture as the one on its own.
    //
    // In the editor they turn its orbit, as the look button does there.
    let look = |yaw: f64, pitch: f64| match editing {
        true => crate::form_view::orbit_from(yaw, pitch),
        false => Action::Look { yaw, pitch },
    };
    if let Some(degrees) = value::<f64>(args, "--turn") {
        actions.push(look(degrees.to_radians(), 0.0));
    }
    if let Some(degrees) = value::<f64>(args, "--pitch") {
        actions.push(look(0.0, degrees.to_radians()));
    }
    // The part the editor's handles and fields are on, by id.
    if let Some(id) = value::<u16>(args, "--select") {
        actions.push(Action::SelectPart(Some(lc_world::form::PartId(id))));
    }
    // Stand-offs toward the nose; both ends are clamps, as the zoom's are.
    if let Some(standoffs) = value::<f64>(args, "--slide") {
        actions.push(Action::SlideForm(standoffs));
    }
    // Both ends of the orbit camera's range are clamps, so the only way to photograph one is
    // to ask for far more than it will give and let it stop where it stops.
    if let Some(notches) = value::<f64>(args, "--zoom") {
        actions.push(Action::Zoom(notches));
    }
    if let Some(name) = after("--map-plane") {
        let plane = match name.as_str() {
            "galactic" => Some(em_map::Plane::Galactic),
            // "ecliptic" was this option's name before a system's plane became something a
            // craft solves; kept so older shot scripts still take the same picture.
            "system" | "ecliptic" => Some(em_map::Plane::System),
            _ => None,
        };
        if let Some(plane) = plane {
            actions.push(Action::SetMapPlane(plane));
        }
    }

    // The sign-in modal draws over the main menu, so it cannot be reached by an action that
    // runs on entering the sky. This is the only way to photograph it.
    let menu_page = (flag("--signin") || flag("--password")).then_some(crate::ui::MenuPage::SignIn);

    // `--menu` holds the entry at the main menu, so `--shot` can photograph it. Without it a
    // screenshot run goes straight to the sky, which is what every other capture wants.
    let stay_in_menu = flag("--menu") || menu_page.is_some();
    let dev = DevEntry {
        observe_immediately: !stay_in_menu
            && (flag("--observe")
                || flag("--shot")
                || flag("--bench")
                || flag("--at")
                || flag("--station")
                || flag("--form")
                || flag("--refit-at")
                || flag("--refit-from")
                || flag("--demo")),
        target_swarm: flag("--swarm"),
        // Not when the camera is pinned: a pin is a request for one exact frame, and turning
        // to face something first would be the aim it exists to stop racing.
        frame_cast: after("--demo").as_deref().and_then(scenario::Scenario::named).is_some() && !flag("--demo-cam"),
        camera: after("--demo-cam").and_then(|spec| {
            let mut fields = spec.split(':').map(|f| f.parse::<f64>());
            match (fields.next(), fields.next(), fields.next()) {
                (Some(Ok(yaw)), Some(Ok(pitch)), Some(Ok(booms))) => Some((yaw, pitch, booms)),
                _ => None,
            }
        }),
        camera_at: after("--demo-cam-at").and_then(|spec| {
            let fields: Vec<f64> = spec.split(':').map(|f| f.parse::<f64>()).collect::<Result<_, _>>().ok()?;
            match fields[..] {
                [x, y, z, m] if m > 0.0 => Some((glam::DVec3::new(x, y, z), m)),
                _ => None,
            }
        }),
        map_focus: after("--map-focus").as_deref().and_then(crate::dev::WantedFocus::named),
        view,
        at_body: after("--at"),
        wear: after("--wear"),
        standoff_radii: value(args, "--standoff"),
        phase_deg: value(args, "--phase"),
        station: after("--station"),
        charted: flag("--charted"),
        beauty_kind: after("--beauty-kind"),
        beauty_dir: after("--beauty-dir"),
        beauty_period_s: value(args, "--beauty-period"),
        map_camera: after("--map").and_then(|spec| {
            let mut fields = spec.split(':').map(|f| f.parse::<f64>());
            match (fields.next(), fields.next(), fields.next()) {
                (Some(Ok(az)), Some(Ok(el)), Some(Ok(au))) => Some((az, el, au)),
                _ => None,
            }
        }),
        lift_deg: value(args, "--lift"),
        form: after("--form"),
        draft: after("--draft"),
        // Only the player's own ship, and no round on the wire yet, so no shard: see
        // `construction`. `--refit-at` on its own asks for the same scene.
        refit: match (value::<f64>(args, "--refit-at"), value::<f64>(args, "--refit-from")) {
            (Some(f), _) => Some(crate::construction::Clock::Frozen(f)),
            (None, Some(f)) => Some(crate::construction::Clock::Looping(f)),
            (None, None) => (after("--demo").as_deref() == Some("refit")).then_some(crate::construction::Clock::Looping(0.0)),
        },
        rate_given: flag("--rate"),
        screenshot: after("--shot"),
        after_frames: value(args, "--frames").unwrap_or(120),
        burst: value(args, "--burst").unwrap_or(1),
        bench: value(args, "--bench"),
        menu_page,
        // The password form is the one egui surface in the menu, and it is opened by a button
        // rather than by a page, so it needs its own way in to be photographed.
        open_password_form: flag("--password"),
        say: after("--say"),
        console: after("--console"),
        actions,
    };
    // The first argument only. Scanning for any non-flag token would pick up a flag's own
    // value: in `--band 2` the `2` looks exactly like a path.
    let catalog = args.first().filter(|a| !a.starts_with("--")).cloned();
    // Asking for a scene is asking for a shard to run it in, so it implies `--local` rather
    // than silently doing nothing without one.
    let demo = after("--demo").filter(|name| scenario::Scenario::named(name).is_some());
    Entry {
        dev,
        catalog,
        server: after("--server"),
        local: flag("--local") || demo.is_some(),
        demo,
    }
}

/// Reads the flags out of a URL query string.
///
/// `?band=2&fly` becomes the same argument vector the desktop binary receives, so [`parse`] is
/// the only thing that knows what a flag means.
///
/// Every parameter is a flag, so the catalog — `parse`'s one positional argument — cannot be
/// named here. The browser build's sky is fixed by the build it belongs to.
#[cfg(target_arch = "wasm32")]
pub fn from_query(query: &str) -> Vec<String> {
    let mut flags = Vec::new();
    for pair in query.trim_start_matches('?').split('&').filter(|p| !p.is_empty()) {
        let (key, value) = match pair.split_once('=') {
            Some((k, v)) => (k, Some(decode(v))),
            None => (pair, None),
        };
        flags.push(format!("--{key}"));
        flags.extend(value);
    }
    flags
}

/// The shard the launching page points this build at.
///
/// From the page rather than the query string, like the ticket and for a related reason: the
/// site knows which shard a build belongs to and the address is part of the handover, not
/// something a player types. A `?server=` flag still wins when one is given, because that is
/// how a build gets pointed at a shard nobody has deployed yet.
#[cfg(target_arch = "wasm32")]
pub fn server_from_page() -> Option<String> {
    web_sys::window()?
        .document()?
        .get_element_by_id("boot")?
        .get_attribute("data-server")
        .filter(|at| !at.is_empty())
}

/// The game ticket the launching page put on `#boot`.
///
/// An attribute and **not** a query parameter, deliberately. A ticket in the URL is a ticket in
/// browser history, in the site's access log, and in a `Referer` if anything on the page is
/// third-party. On an element it is gone when the page is.
///
/// `None` on a deployment with no sign-in, which is the development case and the state this
/// build shipped in.
#[cfg(target_arch = "wasm32")]
pub fn ticket_from_page() -> Option<String> {
    web_sys::window()?
        .document()?
        .get_element_by_id("boot")?
        .get_attribute("data-ticket")
        .filter(|t| !t.is_empty())
}

/// One query parameter, decoded.
///
/// Decoding is not optional: an absolute URL in a parameter comes back percent-encoded, and
/// handing `http%3A%2F%2F...` to the asset server makes it a relative path under the page's
/// own origin. Which is a 404 per asset, and Bevy retries those.
#[cfg(target_arch = "wasm32")]
pub fn param(search: &str, name: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| decode(v))
}

/// Percent-decoding, plus `+` for space. Enough for a query string and no more.
#[cfg(target_arch = "wasm32")]
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = core::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_catalog_is_the_first_argument_and_only_the_first() {
        let cat = parse(&args("sky/hyg-v42.lcsky --band 2")).catalog;
        assert_eq!(cat.as_deref(), Some("sky/hyg-v42.lcsky"));
        // `2` is the band's value, and looks exactly like a path.
        let none = parse(&args("--band 2")).catalog;
        assert_eq!(none, None);
    }

    #[test]
    fn a_form_is_taken_by_name_and_goes_straight_to_the_sky() {
        let dev = parse(&args("--form cluster")).dev;
        assert_eq!(dev.form.as_deref(), Some("cluster"));
        assert!(dev.observe_immediately, "a form asked for is a form to look at");
        assert_eq!(parse(&[]).dev.form, None);
    }

    #[test]
    fn a_shot_implies_observing_immediately() {
        let dev = parse(&args("--shot out.png --frames 90")).dev;
        assert!(dev.observe_immediately);
        assert_eq!(dev.screenshot.as_deref(), Some("out.png"));
        assert_eq!(dev.after_frames, 90);
    }

    #[test]
    fn menu_holds_the_entry_even_when_a_shot_was_asked_for() {
        let dev = parse(&args("--menu --shot menu.png")).dev;
        assert!(!dev.observe_immediately, "--menu must not fall through to the sky");
        assert_eq!(dev.screenshot.as_deref(), Some("menu.png"));
    }

    #[test]
    fn nothing_at_all_is_a_plain_start() {
        let Entry { dev, catalog: cat, server, local, demo } = parse(&[]);
        assert!(!dev.observe_immediately);
        assert!(dev.actions.is_empty());
        assert_eq!(cat, None);
        // No server named is the single-process game, not a default address.
        assert_eq!(server, None);
        assert!(!local);
        assert_eq!(demo, None);
        assert_eq!(dev.camera, None);
    }

    /// A scene needs a shard to run in, so asking for one asks for a shard rather than
    /// silently doing nothing without it.
    #[test]
    fn a_scene_brings_its_own_shard() {
        let entry = parse(&args("--demo chase"));
        assert_eq!(entry.demo.as_deref(), Some("chase"));
        assert!(entry.local, "a scene was asked for with nowhere to run it");
        assert!(entry.dev.observe_immediately, "it stopped at the menu");
    }

    /// A name nobody has is no name at all. Better a plain start than a shard staging silence.
    #[test]
    fn a_scene_nobody_has_is_not_taken() {
        assert_eq!(parse(&args("--demo nonesuch")).demo, None);
        assert!(!parse(&args("--demo nonesuch")).local);
    }

    /// The camera pin is three numbers, and anything else is not a pin.
    #[test]
    fn a_pinned_camera_is_read_whole_or_not_at_all() {
        assert_eq!(parse(&args("--demo-cam 90:20:6")).dev.camera, Some((90.0, 20.0, 6.0)));
        assert_eq!(parse(&args("--demo-cam 90:20")).dev.camera, None);
        assert_eq!(parse(&args("--demo-cam what")).dev.camera, None);
    }

    /// `--local` is its own thing, not an address, because the port is not known until the
    /// socket is bound.
    #[test]
    fn a_local_shard_is_asked_for_rather_than_addressed() {
        assert!(parse(&args("--local")).local);
        assert_eq!(parse(&args("--local")).server, None);
        assert!(!parse(&args("--server ws://host:1/")).local);
    }

    #[test]
    fn a_server_is_taken_from_the_flag_and_not_guessed() {
        assert_eq!(
            parse(&args("--server ws://127.0.0.1:8080")).server.as_deref(),
            Some("ws://127.0.0.1:8080"),
        );
        // And it is not mistaken for the catalog, which is the first positional argument.
        let entry = parse(&args("sky/hyg-v42.lcsky --server ws://host:1/"));
        assert_eq!(entry.catalog.as_deref(), Some("sky/hyg-v42.lcsky"));
        assert_eq!(entry.server.as_deref(), Some("ws://host:1/"));
    }
}
