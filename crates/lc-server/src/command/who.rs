//! `who-is`: what the shard holds about a ship, by its id or its name.

use std::fmt::Write as _;

use lc_world::craft::{Craft, CraftId};

use crate::journal::Journal;
use crate::server::Server;

impl<J: Journal> Server<J> {
    /// By name, every match: nothing makes a name unique.
    pub(super) fn who_is(&self, id: Option<u64>, name: Option<&str>) -> Result<String, String> {
        let found: Vec<&Craft> = match (id, name) {
            (Some(raw), None) => {
                let craft = i64::try_from(raw).ok().and_then(|id| self.fleet.get(CraftId(id)));
                vec![craft.ok_or_else(|| format!("no ship {raw}"))?]
            }
            (None, Some(name)) => {
                let mut found: Vec<&Craft> =
                    self.fleet.iter().filter(|c| c.designation().eq_ignore_ascii_case(name)).collect();
                found.sort_by_key(|c| c.id.0);
                if found.is_empty() {
                    return Err(format!("no ship is called {name}"));
                }
                found
            }
            (Some(_), Some(_)) => return Err("an id: or a name:, not both".into()),
            (None, None) => return Err("an id: or a name: is required".into()),
        };
        let mut out = String::new();
        for craft in found {
            if !out.is_empty() {
                out.push('\n');
            }
            let _ = write!(out, "ship {}\n  name: {}", craft.id.0, craft.designation());
        }
        Ok(out)
    }
}
