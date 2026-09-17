//! Conversations: what was said, what landed, and who holds whose key.
//!
//! The rest of this crate answers "what can this observer see by now", which is a question
//! about geometry and is asked twenty times a second. This answers "what has this ship said and
//! been told", which is a question about who was talking and is asked once, when somebody signs
//! in. The two want different tables and `sql/0005_chat.sql` says why at length.
//!
//! No opinion here about what a message *means*. A body is text this crate never reads, sealing
//! is a boolean it never acts on, and redacting one for a receiver who may not read it is the
//! server's business — by the time a row reaches here it is a record, not a decision.

use tokio_postgres::{Client, Error};

/// One transmission, whole. The body is never redacted in the store.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub event_id: i64,
    pub sender: i64,
    /// Who it was addressed to, which is not who heard it.
    pub addressee: i64,
    pub sealed: bool,
    /// A key offer: a message with nothing in it, kept in the same transcript because that is
    /// where a player looks for it.
    pub is_key: bool,
    pub body: String,
    /// Event ids of the addressee's messages the sender had received when this went out.
    pub acks: Vec<i64>,
    pub sent_t: i64,
    /// Which message this is, across its resends. `None` for a row written before the column
    /// existed; see `sql/0006_message_key.sql` for why that is not a zero.
    pub idem: Option<i64>,
}

/// A transmission landing on somebody: the addressee, or anyone else in earshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Receipt {
    pub event_id: i64,
    pub observer: i64,
    pub arrive_t: i64,
}

/// One ship holding another's public key, as of the moment the offer's light arrived.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Held {
    pub holder: i64,
    pub subject: i64,
    pub learnt_t: i64,
}

/// How much of a transcript a sign-in is handed.
///
/// A cap and not a policy: everything is still in the table, and a conversation that has run to
/// more than this wants a window the interface asks for rather than a larger constant here.
pub const BACKLOG_LIMIT: i64 = 500;

/// Write messages. One statement each, because a message carries an array of acknowledgements
/// and `unnest` flattens a two-dimensional one into a single column.
///
/// Cheap regardless: these arrive at the rate a person types, not at the rate the world ticks.
pub async fn save_messages(client: &Client, messages: &[Message]) -> Result<u64, Error> {
    let mut written = 0;
    for m in messages {
        written += client
            .execute(
                "INSERT INTO lc_messages
                     (event_id, sender, addressee, sealed, is_key, body, acks, sent_t, idem)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT (event_id) DO NOTHING",
                &[
                    &m.event_id,
                    &m.sender,
                    &m.addressee,
                    &m.sealed,
                    &m.is_key,
                    &m.body,
                    &m.acks,
                    &m.sent_t,
                    &m.idem,
                ],
            )
            .await?;
    }
    Ok(written)
}

/// Write receipts. One statement whatever the count: a loud message lands on everyone in range.
pub async fn save_receipts(client: &Client, receipts: &[Receipt]) -> Result<u64, Error> {
    if receipts.is_empty() {
        return Ok(0);
    }
    let events: Vec<i64> = receipts.iter().map(|r| r.event_id).collect();
    let observers: Vec<i64> = receipts.iter().map(|r| r.observer).collect();
    let arrivals: Vec<i64> = receipts.iter().map(|r| r.arrive_t).collect();
    client
        .execute(
            "INSERT INTO lc_message_receipts (event_id, observer, arrive_t)
             SELECT * FROM unnest($1::bigint[], $2::bigint[], $3::bigint[])
             ON CONFLICT (observer, event_id) DO NOTHING",
            &[&events, &observers, &arrivals],
        )
        .await
}

/// Write keyring rows. The first offer to arrive is the one that counts: a second copy of a key
/// somebody already holds teaches them nothing, and would move the date they learnt it.
pub async fn save_keys(client: &Client, keys: &[Held]) -> Result<u64, Error> {
    if keys.is_empty() {
        return Ok(0);
    }
    let holders: Vec<i64> = keys.iter().map(|k| k.holder).collect();
    let subjects: Vec<i64> = keys.iter().map(|k| k.subject).collect();
    let learnt: Vec<i64> = keys.iter().map(|k| k.learnt_t).collect();
    client
        .execute(
            "INSERT INTO lc_keyring (holder, subject, learnt_t)
             SELECT * FROM unnest($1::bigint[], $2::bigint[], $3::bigint[])
             ON CONFLICT (holder, subject) DO NOTHING",
            &[&holders, &subjects, &learnt],
        )
        .await
}

fn message_from(row: &tokio_postgres::Row) -> Message {
    Message {
        event_id: row.get(0),
        sender: row.get(1),
        addressee: row.get(2),
        sealed: row.get(3),
        is_key: row.get(4),
        body: row.get(5),
        acks: row.get(6),
        sent_t: row.get(7),
        idem: row.get(8),
    }
}

const COLUMNS: &str = "event_id, sender, addressee, sealed, is_key, body, acks, sent_t, idem";

/// Everything this ship transmitted, oldest first.
pub async fn sent_by(client: &Client, ship: i64) -> Result<Vec<Message>, Error> {
    // Newest by the index, then turned round: the tail of a long conversation is what a
    // sign-in wants, and ordering ascending would make the limit take the wrong end.
    let rows = client
        .query(
            &format!(
                "SELECT {COLUMNS} FROM lc_messages
                  WHERE sender = $1 ORDER BY sent_t DESC LIMIT $2"
            ),
            &[&ship, &BACKLOG_LIMIT],
        )
        .await?;
    let mut out: Vec<Message> = rows.iter().map(message_from).collect();
    out.reverse();
    Ok(out)
}

/// Everything that reached this ship, oldest arrival first, with when it landed.
pub async fn heard_by(client: &Client, ship: i64) -> Result<Vec<(Message, i64)>, Error> {
    let rows = client
        .query(
            &format!(
                "SELECT {}, r.arrive_t
                   FROM lc_message_receipts r JOIN lc_messages m USING (event_id)
                  WHERE r.observer = $1 ORDER BY r.arrive_t DESC LIMIT $2",
                COLUMNS.split(", ").map(|c| format!("m.{c}")).collect::<Vec<_>>().join(", ")
            ),
            &[&ship, &BACKLOG_LIMIT],
        )
        .await?;
    let mut out: Vec<(Message, i64)> =
        rows.iter().map(|row| (message_from(row), row.get(9))).collect();
    out.reverse();
    Ok(out)
}

/// Whose keys this ship holds.
pub async fn keys_of(client: &Client, ship: i64) -> Result<Vec<i64>, Error> {
    let rows = client
        .query("SELECT subject FROM lc_keyring WHERE holder = $1 ORDER BY learnt_t", &[&ship])
        .await?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// Every keyring row there is.
///
/// Whole rather than per ship, because a shard coming back needs all of them at once and there
/// is one row per pair of craft that have ever exchanged a key — a far smaller number than the
/// messages that carried them.
pub async fn all_keys(client: &Client) -> Result<Vec<Held>, Error> {
    let rows = client
        .query("SELECT holder, subject, learnt_t FROM lc_keyring", &[])
        .await?;
    Ok(rows
        .iter()
        .map(|row| Held { holder: row.get(0), subject: row.get(1), learnt_t: row.get(2) })
        .collect())
}

/// The last `depth` messages each observer received from each sender, oldest first.
///
/// What a reply acknowledges, rebuilt after a restart. Without it the first message anybody
/// sends when a shard comes back acknowledges nothing, and the other end reads that as its own
/// messages having been lost — which is the one thing an acknowledgement exists to rule out.
pub async fn recent_heard(client: &Client, depth: i64) -> Result<Vec<(i64, i64, i64)>, Error> {
    let rows = client
        .query(
            "SELECT observer, sender, event_id FROM (
                 SELECT r.observer, m.sender, r.event_id, r.arrive_t,
                        row_number() OVER (PARTITION BY r.observer, m.sender
                                           ORDER BY r.arrive_t DESC) AS n
                   FROM lc_message_receipts r JOIN lc_messages m USING (event_id)
                  WHERE m.is_key = false
             ) ranked
              WHERE n <= $1
              ORDER BY observer, sender, arrive_t",
            &[&depth],
        )
        .await?;
    Ok(rows.iter().map(|row| (row.get(0), row.get(1), row.get(2))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client against a store with the schema applied, or `None` where there is no database.
    /// The suite has to pass without one; see AGENTS.md.
    async fn store() -> Option<Client> {
        let client = crate::connect().await.ok()?;
        crate::migrate::apply(&client).await.ok()?;
        Some(client)
    }

    /// Identifiers well clear of anything another test would pick, so the two can run at once.
    fn scratch(tag: i64) -> (i64, i64) {
        (900_000 + tag * 10, 900_001 + tag * 10)
    }

    async fn clear(client: &Client, ships: (i64, i64)) {
        let ids = vec![ships.0, ships.1];
        let _ = client
            .execute("DELETE FROM lc_messages WHERE sender = ANY($1)", &[&ids])
            .await;
        let _ = client
            .execute("DELETE FROM lc_message_receipts WHERE observer = ANY($1)", &[&ids])
            .await;
        let _ = client.execute("DELETE FROM lc_keyring WHERE holder = ANY($1)", &[&ids]).await;
    }

    fn message(event_id: i64, from: i64, to: i64, body: &str, sent_t: i64) -> Message {
        Message {
            event_id,
            sender: from,
            addressee: to,
            sealed: false,
            is_key: false,
            body: body.into(),
            acks: Vec::new(),
            sent_t,
            idem: Some(event_id),
        }
    }

    #[tokio::test]
    async fn a_ship_reads_back_both_halves_of_its_own_conversation() {
        let Some(client) = store().await else { return };
        let (ada, bry) = scratch(1);
        clear(&client, (ada, bry)).await;

        let out = message(910_001, ada, bry, "are you there", 1_000);
        let back = Message { acks: vec![910_001], ..message(910_002, bry, ada, "here", 4_000) };
        save_messages(&client, &[out, back]).await.unwrap();
        save_receipts(&client, &[
            Receipt { event_id: 910_001, observer: bry, arrive_t: 3_000 },
            Receipt { event_id: 910_002, observer: ada, arrive_t: 6_000 },
        ])
        .await
        .unwrap();

        let mine = sent_by(&client, ada).await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].body, "are you there");

        let heard = heard_by(&client, ada).await.unwrap();
        assert_eq!(heard.len(), 1, "a sender does not hear its own signal");
        assert_eq!(heard[0].0.body, "here");
        assert_eq!(heard[0].0.acks, vec![910_001], "the acknowledgement survived the array");
        assert_eq!(heard[0].1, 6_000);

        clear(&client, (ada, bry)).await;
    }

    /// An eavesdropper's receipt is a row like anybody else's. The store does not know that the
    /// message was not for them, and must not: who may *read* it is decided on release.
    #[tokio::test]
    async fn a_message_lands_on_everyone_in_earshot_and_not_only_its_addressee() {
        let Some(client) = store().await else { return };
        let (ada, bry) = scratch(2);
        let nosy = bry + 1;
        clear(&client, (ada, bry)).await;
        let _ = client.execute("DELETE FROM lc_message_receipts WHERE observer = $1", &[&nosy]).await;

        save_messages(&client, &[message(920_001, ada, bry, "in the open", 1_000)]).await.unwrap();
        save_receipts(&client, &[
            Receipt { event_id: 920_001, observer: bry, arrive_t: 2_000 },
            Receipt { event_id: 920_001, observer: nosy, arrive_t: 2_500 },
        ])
        .await
        .unwrap();

        assert_eq!(heard_by(&client, nosy).await.unwrap().len(), 1);
        clear(&client, (ada, bry)).await;
        let _ = client.execute("DELETE FROM lc_message_receipts WHERE observer = $1", &[&nosy]).await;
    }

    /// The ack window, rebuilt: newest `depth` per pair, oldest first, key offers excluded.
    #[tokio::test]
    async fn the_ack_window_comes_back_newest_first_and_bounded() {
        let Some(client) = store().await else { return };
        let (ada, bry) = scratch(3);
        clear(&client, (ada, bry)).await;

        let mut messages = Vec::new();
        let mut receipts = Vec::new();
        for k in 0..15i64 {
            let id = 930_000 + k;
            messages.push(message(id, bry, ada, "tick", 1_000 + k));
            receipts.push(Receipt { event_id: id, observer: ada, arrive_t: 2_000 + k });
        }
        // A key offer in the middle: it is in the transcript and never in the ack window.
        messages.push(Message { is_key: true, ..message(930_100, bry, ada, "", 1_500) });
        receipts.push(Receipt { event_id: 930_100, observer: ada, arrive_t: 2_500 });
        save_messages(&client, &messages).await.unwrap();
        save_receipts(&client, &receipts).await.unwrap();

        let window: Vec<i64> = recent_heard(&client, 10)
            .await
            .unwrap()
            .into_iter()
            .filter(|(observer, sender, _)| *observer == ada && *sender == bry)
            .map(|(_, _, event)| event)
            .collect();
        assert_eq!(window.len(), 10);
        assert_eq!(window.first(), Some(&930_005), "the oldest of the newest ten, first");
        assert_eq!(window.last(), Some(&930_014));
        assert!(!window.contains(&930_100), "a key offer is not something to acknowledge");

        clear(&client, (ada, bry)).await;
    }

    #[tokio::test]
    async fn a_key_is_learnt_once_and_the_date_does_not_move() {
        let Some(client) = store().await else { return };
        let (ada, bry) = scratch(4);
        clear(&client, (ada, bry)).await;

        save_keys(&client, &[Held { holder: ada, subject: bry, learnt_t: 100 }]).await.unwrap();
        save_keys(&client, &[Held { holder: ada, subject: bry, learnt_t: 900 }]).await.unwrap();
        let held: Vec<Held> =
            all_keys(&client).await.unwrap().into_iter().filter(|k| k.holder == ada).collect();
        assert_eq!(held, vec![Held { holder: ada, subject: bry, learnt_t: 100 }]);

        clear(&client, (ada, bry)).await;
    }
}
