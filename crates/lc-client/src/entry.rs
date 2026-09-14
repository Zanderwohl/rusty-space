//! Turning a request to start the client into a [`DevEntry`].
//!
//! Shared by the desktop binary, which gets its request from `argv`, and the browser one,
//! which gets it from the query string. The two differ only in where the strings come from,
//! and a second copy of this parsing would be a second set of flag names.

use crate::action::Action;
use crate::app::DevEntry;

/// Parses the flag vocabulary both binaries accept.
///
/// Returns the entry and the catalogue to load: the first positional argument, which is an
/// asset path rather than a filesystem one.
pub fn parse(args: &[String]) -> (DevEntry, Option<String>) {
    let flag = |name: &str| args.iter().any(|a| a == name);
    let after = |name: &str| {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
    };
    fn value<T: std::str::FromStr>(args: &[String], name: &str) -> Option<T> {
        let i = args.iter().position(|a| a == name)?;
        args.get(i + 1)?.parse().ok()
    }

    let mut actions = Vec::new();
    if let Some(preset) = value::<usize>(args, "--band") {
        actions.push(Action::SetBandPreset(preset));
    }
    if let Some(rate) = value::<f64>(args, "--rate") {
        actions.push(Action::SetTimeRate(rate));
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
    if let Some(b) = value::<usize>(args, "--curve")
        && let Some(band) = em_spectra::Band::ALL.get(b)
    {
        actions.push(Action::SetCurveBand(*band));
    }
    if flag("--fly") {
        // Index 0 of the sorted sky is the Sun in the full catalogue; 1 is interstellar.
        actions.push(Action::FlyToNearest);
    }

    let dev = DevEntry {
        observe_immediately: flag("--observe")
            || flag("--shot")
            || flag("--at")
            || flag("--station"),
        target_swarm: flag("--swarm"),
        at_body: after("--at"),
        station: after("--station"),
        screenshot: after("--shot"),
        after_frames: value(args, "--frames").unwrap_or(120),
        burst: value(args, "--burst").unwrap_or(1),
        actions,
    };
    // The first argument only. Scanning for any non-flag token would pick up a flag's own
    // value: in `--band 2` the `2` looks exactly like a path.
    let catalogue = args.first().filter(|a| !a.starts_with("--")).cloned();
    (dev, catalogue)
}

/// Reads the flags out of a URL query string.
///
/// `?band=2&fly&sky=sky/hyg-v42.lcsky` becomes the same argument vector the desktop binary
/// receives, so [`parse`] is the only thing that knows what a flag means.
#[cfg(target_arch = "wasm32")]
pub fn from_query(query: &str) -> Vec<String> {
    let mut flags = Vec::new();
    let mut positional = Vec::new();
    for pair in query.trim_start_matches('?').split('&').filter(|p| !p.is_empty()) {
        let (key, value) = match pair.split_once('=') {
            Some((k, v)) => (k, Some(decode(v))),
            None => (pair, None),
        };
        // `sky` is the catalogue, and is the one parameter that is not a flag.
        if key == "sky" {
            if let Some(v) = value {
                positional.push(v);
            }
            continue;
        }
        flags.push(format!("--{key}"));
        if let Some(v) = value {
            flags.push(v);
        }
    }
    // The catalogue leads, because that is where `parse` looks for it.
    positional.extend(flags);
    positional
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
    fn the_catalogue_is_the_first_argument_and_only_the_first() {
        let (_, cat) = parse(&args("sky/hyg-v42.lcsky --band 2"));
        assert_eq!(cat.as_deref(), Some("sky/hyg-v42.lcsky"));
        // `2` is the band's value, and looks exactly like a path.
        let (_, none) = parse(&args("--band 2"));
        assert_eq!(none, None);
    }

    #[test]
    fn a_shot_implies_observing_immediately() {
        let (dev, _) = parse(&args("--shot out.png --frames 90"));
        assert!(dev.observe_immediately);
        assert_eq!(dev.screenshot.as_deref(), Some("out.png"));
        assert_eq!(dev.after_frames, 90);
    }

    #[test]
    fn nothing_at_all_is_a_plain_start() {
        let (dev, cat) = parse(&[]);
        assert!(!dev.observe_immediately);
        assert!(dev.actions.is_empty());
        assert_eq!(cat, None);
    }
}
