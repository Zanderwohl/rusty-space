//! The administrative log: who changed what about whom.
//!
//! Append-only. A promotion with nothing recording it is a power that appeared from nowhere
//! and a demotion one that vanished; neither is recoverable from the accounts table, which
//! only ever holds the present.
//!
//! `detail` is rendered when the act happens, so renaming a level does not rewrite history.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::store::{Store, StoreError};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Promoted,
    Demoted,
    Banned,
    Lifted,
}

impl Action {
    pub const ALL: [Action; 4] = [
        Action::Promoted,
        Action::Demoted,
        Action::Banned,
        Action::Lifted,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Action::Promoted => "promoted",
            Action::Demoted => "demoted",
            Action::Banned => "banned",
            Action::Lifted => "lifted",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Promoted => "Promoted",
            Action::Demoted => "Demoted",
            Action::Banned => "Banned",
            Action::Lifted => "Ban lifted",
        }
    }

    /// Unrecognised names render as themselves; see [`crate::bans::Reason::from_stored`].
    pub fn from_stored(stored: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.slug() == stored)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub actor_id: Option<Uuid>,
    pub actor_name: Option<String>,
    pub subject_id: Option<Uuid>,
    pub subject_name: Option<String>,
    /// `None` for a name this build does not know; [`Entry::raw`] keeps it.
    pub action: Option<Action>,
    pub raw: String,
    pub detail: String,
    pub at: DateTime<Utc>,
}

impl Entry {
    pub fn label(&self) -> &str {
        match self.action {
            Some(action) => action.label(),
            None => self.raw.as_str(),
        }
    }
}

impl Store {
    /// Best effort at the call site: an administrative change that succeeded must not be reported
    /// as failed because the log write did.
    pub async fn record(
        &self,
        actor: Uuid,
        subject: Uuid,
        action: Action,
        detail: &str,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                let mut tables = m.0.lock().unwrap();
                let id = tables.actions.len() as i64 + 1;
                tables.actions.push(Entry {
                    id,
                    actor_id: Some(actor),
                    actor_name: None,
                    subject_id: Some(subject),
                    subject_name: None,
                    action: Some(action),
                    raw: action.slug().to_owned(),
                    detail: detail.to_owned(),
                    at: now,
                });
                Ok(())
            }
            Store::Postgres(pool) => {
                sqlx::query(
                    "insert into admin_actions (actor_id, subject_id, action, detail, at) \
                     values ($1, $2, $3, $4, $5)",
                )
                .bind(actor)
                .bind(subject)
                .bind(action.slug())
                .bind(detail)
                .bind(now)
                .execute(pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
                Ok(())
            }
        }
    }

    pub async fn actions_for(&self, subject: Uuid, limit: i64) -> Result<Vec<Entry>, StoreError> {
        match self {
            Store::Memory(m) => {
                let tables = m.0.lock().unwrap();
                let mut found: Vec<Entry> = tables
                    .actions
                    .iter()
                    .filter(|e| e.subject_id == Some(subject))
                    .cloned()
                    .collect();
                found.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
                found.truncate(limit.max(0) as usize);
                Ok(found)
            }
            Store::Postgres(pool) => {
                type Row = (
                    i64,
                    Option<Uuid>,
                    Option<String>,
                    Option<Uuid>,
                    Option<String>,
                    String,
                    String,
                    DateTime<Utc>,
                );
                let rows: Vec<Row> = sqlx::query_as(
                    "select a.id, a.actor_id, actor.display_name, \
                            a.subject_id, subject.display_name, a.action, a.detail, a.at \
                     from admin_actions a \
                     left join accounts actor on actor.id = a.actor_id \
                     left join accounts subject on subject.id = a.subject_id \
                     where a.subject_id = $1 \
                     order by a.at desc, a.id desc \
                     limit $2",
                )
                .bind(subject)
                .bind(limit)
                .fetch_all(pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
                Ok(rows
                    .into_iter()
                    .map(|r| Entry {
                        id: r.0,
                        actor_id: r.1,
                        actor_name: r.2,
                        subject_id: r.3,
                        subject_name: r.4,
                        action: Action::from_stored(&r.5),
                        raw: r.5,
                        detail: r.6,
                        at: r.7,
                    })
                    .collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_log_reads_back_newest_first() {
        let store = Store::memory();
        let actor = Uuid::new_v4();
        let subject = Uuid::new_v4();
        let at = Utc::now();
        store
            .record(actor, subject, Action::Promoted, "Player to Moderator", at)
            .await
            .unwrap();
        store
            .record(
                actor,
                subject,
                Action::Demoted,
                "Moderator to Player",
                at + chrono::Duration::hours(1),
            )
            .await
            .unwrap();

        let log = store.actions_for(subject, 10).await.unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].action, Some(Action::Demoted));
        assert_eq!(log[0].detail, "Moderator to Player");
        assert_eq!(log[1].action, Some(Action::Promoted));
        assert_eq!(log[0].label(), "Demoted");

        // Somebody else's account shares nothing with this one.
        assert!(
            store
                .actions_for(Uuid::new_v4(), 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_unknown_action_still_renders() {
        let entry = Entry {
            id: 1,
            actor_id: None,
            actor_name: None,
            subject_id: None,
            subject_name: None,
            action: Action::from_stored("renamed"),
            raw: "renamed".into(),
            detail: String::new(),
            at: Utc::now(),
        };
        assert_eq!(entry.action, None);
        assert_eq!(entry.label(), "renamed");
        for action in Action::ALL {
            assert_eq!(Action::from_stored(action.slug()), Some(action));
        }
    }
}
