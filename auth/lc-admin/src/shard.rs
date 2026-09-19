//! Asking the game where somebody's ship is.
//!
//! **These types are a copy, and they are supposed to be.** The originals are
//! `lc_server::status`, in the game's cargo workspace, which this service may not depend on —
//! `auth/` is separate from the products and CI refuses a path dependency across the line.
//! What crosses instead is **RON**: a format, not a type, so the shard serialises its structs
//! and this deserialises into these.
//!
//! The cost is stated plainly because it is real: **a field renamed on the far side stops
//! arriving here**, and nothing in either build will say so. What catches it is
//! `parses_what_the_shard_sends`, which holds a captured payload — change the shape over
//! there and that test is where it surfaces.
//!
//! The same trade as the ticket claims, for the same reason, and
//! `lightcone/docs/16-identity.md` is where the reason is written down.

use serde::Deserialize;

/// Where a craft is. Mirrors `lc_server::status::Whereabouts`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub enum Whereabouts {
    In { star: u64, name: String, au: f64 },
    Interstellar { star: u64, near: String, ly: f64 },
    Nowhere,
}

/// Mirrors `lc_server::status::Fit`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Fit {
    pub modules: Vec<(String, u32)>,
    pub hull_slots: u32,
    pub used_slots: u32,
    pub stored_j: f64,
    pub solar_w: f64,
    pub committed_j: f64,
    pub refitting: bool,
}

/// Mirrors `lc_server::status::Status`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Status {
    pub ship_id: i64,
    pub name: Option<String>,
    pub length_m: f64,
    pub saved_t: i64,
    pub whereabouts: Whereabouts,
    pub fit: Option<Fit>,
}

/// Why there is nothing to show.
///
/// A card that says which of these is a card somebody can act on; one that says "unavailable"
/// for all four is a card that trains people to ignore it.
#[derive(Clone, Debug, PartialEq)]
pub enum Missing {
    /// No shard is configured. The console runs without one.
    NotConfigured,
    /// The account has never been in the world.
    NoShip,
    /// The shard did not answer, or answered with something unreadable.
    Unreachable(String),
}

impl Missing {
    pub fn said(&self) -> String {
        match self {
            Missing::NotConfigured => "This console is not pointed at a shard.".to_owned(),
            Missing::NoShip => "This account has never entered the world.".to_owned(),
            Missing::Unreachable(why) => format!("The shard did not answer: {why}"),
        }
    }
}

/// What it takes to ask.
pub struct Shard<'a> {
    /// Where the shard's administration surface is, on the container network.
    pub api: &'a str,
    /// The audience its tickets must name.
    pub audience: &'a str,
    /// The broker, and the secret that lets this service mint a ticket on an administrator's
    /// behalf.
    pub identity_api: &'a str,
    pub identity_secret: &'a str,
    pub http: &'a reqwest::Client,
}

#[derive(Deserialize)]
struct Minted {
    ticket: String,
}

impl Shard<'_> {
    /// One account's ship, as the shard last checkpointed it.
    ///
    /// Two calls: mint a ticket for the **acting administrator**, then present it. Minting per
    /// request rather than holding one is not waste — a ticket is sixty seconds and single
    /// use, so there is nothing to hold, and it means the shard's log says who was looking.
    pub async fn status(&self, acting: &str, about: &str) -> Result<Status, Missing> {
        let ticket = self.ticket_for(acting).await?;
        let response = self
            .http
            .get(format!("{}/admin/status/{about}", self.api))
            .bearer_auth(&ticket)
            .send()
            .await
            .map_err(|why| Missing::Unreachable(why.to_string()))?;

        match response.status() {
            s if s.is_success() => {}
            reqwest::StatusCode::NOT_FOUND => return Err(Missing::NoShip),
            other => return Err(Missing::Unreachable(format!("it answered {other}"))),
        }
        let body = response
            .text()
            .await
            .map_err(|why| Missing::Unreachable(why.to_string()))?;
        ron::from_str(&body).map_err(|why| {
            // The shape changed on the far side, most likely. Logged with the body so the
            // next person does not have to reproduce it to find out what arrived.
            tracing::error!(%why, %body, "the shard's status did not parse");
            Missing::Unreachable("it answered in a shape this build does not read".to_owned())
        })
    }

    /// A ticket for the administrator whose page this is.
    ///
    /// The broker's `/ticket`, the same endpoint the website calls to put a player into the
    /// game, taking the same shared secret. The ticket carries that administrator's level,
    /// which is what the shard gates on.
    async fn ticket_for(&self, account: &str) -> Result<String, Missing> {
        let response = self
            .http
            .post(format!("{}/ticket", self.identity_api))
            .bearer_auth(self.identity_secret)
            .json(&serde_json::json!({
                "account_id": account,
                "audience": self.audience,
            }))
            .send()
            .await
            .map_err(|why| Missing::Unreachable(format!("no ticket: {why}")))?;
        if !response.status().is_success() {
            return Err(Missing::Unreachable(format!(
                "the broker would not mint a ticket: {}",
                response.status()
            )));
        }
        response
            .json::<Minted>()
            .await
            .map(|m| m.ticket)
            .map_err(|why| Missing::Unreachable(format!("no ticket: {why}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A captured payload — the bytes `lc_server::status` actually produced**, not a
    /// hand-written approximation of them. It was printed by that crate's own serialiser and
    /// pasted here, which is the only way this is evidence of anything: a sample written from
    /// memory agrees with whatever the author believed and proves nothing about the far side.
    ///
    /// This is the contract. If the shard renames a field, this string stops matching the
    /// structs above and this test is the only place in either build that will say so —
    /// there is no compiler between the two, by design.
    const FROM_THE_SHARD: &str = r#"(ship_id:7,name:Some("Rocinante"),length_m:46.0,saved_t:1234567,whereabouts:In(star:1,name:"Sol",au:5.0),fit:Some((modules:[("Engines",2),("Storage",1),("Drone bays",0),("Living",1)],hull_slots:8,used_slots:4,stored_j:1500000000000.0,solar_w:4200.0,committed_j:0.0,refitting:false)))"#;

    #[test]
    fn parses_what_the_shard_sends() {
        let status: Status = ron::from_str(FROM_THE_SHARD).expect("the shard's shape");
        assert_eq!(status.ship_id, 7);
        assert_eq!(status.name.as_deref(), Some("Rocinante"));
        assert_eq!(
            status.whereabouts,
            Whereabouts::In {
                star: 1,
                name: "Sol".into(),
                au: 5.0,
            },
        );
        let fit = status.fit.expect("a fitting");
        assert_eq!(fit.hull_slots, 8);
        assert_eq!(fit.modules.len(), 4);
        assert_eq!(fit.modules[0], ("Engines".to_owned(), 2));
        // Exactly, which is half the reason this is RON rather than JSON.
        assert_eq!(fit.stored_j, 1.5e12);
    }

    /// The other two shapes the shard can send.
    #[test]
    fn parses_a_craft_between_the_stars_and_one_with_no_fitting() {
        let between: Status = ron::from_str(
            r#"(ship_id: 1, name: None, length_m: 12.0, saved_t: 0,
                whereabouts: Interstellar(star: 9, near: "Sol", ly: 2.4), fit: None)"#,
        )
        .expect("interstellar");
        assert_eq!(
            between.whereabouts,
            Whereabouts::Interstellar {
                star: 9,
                near: "Sol".into(),
                ly: 2.4,
            },
        );
        assert!(between.fit.is_none());
        assert!(between.name.is_none());

        let nowhere: Status = ron::from_str(
            r#"(ship_id: 1, name: None, length_m: 12.0, saved_t: 0,
                whereabouts: Nowhere, fit: None)"#,
        )
        .expect("nowhere");
        assert_eq!(nowhere.whereabouts, Whereabouts::Nowhere);
    }

    /// Each absence says which absence it is. One message for all of them is a card people
    /// learn to stop reading.
    #[test]
    fn every_absence_says_something_different() {
        let said: Vec<String> = [
            Missing::NotConfigured,
            Missing::NoShip,
            Missing::Unreachable("connection refused".into()),
        ]
        .iter()
        .map(Missing::said)
        .collect();
        assert!(said.iter().all(|s| !s.is_empty()));
        let mut unique = said.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            said.len(),
            "two absences read alike: {said:?}"
        );
        assert!(said[2].contains("connection refused"), "{}", said[2]);
    }
}
