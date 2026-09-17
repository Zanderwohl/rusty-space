//! Applying the schema.
//!
//! The SQL is compiled in rather than read from disk, so a binary carries its own schema and a
//! test needs no working directory. Each step runs once, inside a transaction, and is recorded.

use tokio_postgres::Client;

/// Every step, in order. The name is the record; never rename or reorder one that has shipped.
pub const STEPS: &[(&str, &str)] =
    &[
        ("0001_causality", include_str!("../sql/0001_causality.sql")),
        ("0002_schema", include_str!("../sql/0002_schema.sql")),
        ("0003_retention", include_str!("../sql/0003_retention.sql")),
        ("0004_ships", include_str!("../sql/0004_ships.sql")),
        ("0005_chat", include_str!("../sql/0005_chat.sql")),
    ];

/// The advisory lock every migrator takes before touching the schema.
///
/// Arbitrary, and fixed forever: it only has to be a number no other user of this database
/// picks. Without it two processes starting together both find the step table missing and both
/// create it, and one of them fails on a duplicate key in `pg_type` — which is what two servers
/// coming up at once looks like, and what three tests running in parallel found first.
pub const SCHEMA_LOCK: i64 = 0x1c_57_04_3e; // "lc store"

/// Apply whatever has not been applied. Idempotent, and safe to run from several processes.
pub async fn apply(client: &Client) -> Result<Vec<&'static str>, tokio_postgres::Error> {
    client.execute("SELECT pg_advisory_lock($1)", &[&SCHEMA_LOCK]).await?;
    let result = apply_locked(client).await;
    // Released whether or not the steps worked: holding it past a failure would wedge every
    // other process against a problem they cannot fix.
    client.execute("SELECT pg_advisory_unlock($1)", &[&SCHEMA_LOCK]).await?;
    result
}

async fn apply_locked(client: &Client) -> Result<Vec<&'static str>, tokio_postgres::Error> {
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS lc_schema_steps (
                 name       text PRIMARY KEY,
                 applied_at timestamptz NOT NULL DEFAULT now()
             )",
        )
        .await?;

    let mut ran = Vec::new();
    for (name, sql) in STEPS {
        let done = client
            .query_opt("SELECT 1 FROM lc_schema_steps WHERE name = $1", &[name])
            .await?
            .is_some();
        if done {
            continue;
        }
        // One transaction per step: a step that fails half way leaves nothing behind, and the
        // record of it only exists if the whole of it ran.
        client.batch_execute("BEGIN").await?;
        match client.batch_execute(sql).await {
            Ok(()) => {}
            Err(error) => {
                let _ = client.batch_execute("ROLLBACK").await;
                return Err(error);
            }
        }
        client.execute("INSERT INTO lc_schema_steps (name) VALUES ($1)", &[name]).await?;
        client.batch_execute("COMMIT").await?;
        ran.push(*name);
    }
    Ok(ran)
}

#[cfg(test)]
mod tests {
    use lc_spacetime::coord::{COORD_BOUND, Coord};
    use lc_spacetime::interval::{Separation, classify, precedes};
    use lc_spacetime::units::Micros;

    use super::*;

    async fn sql_precedes(client: &Client, a: Coord, b: Coord) -> bool {
        client
            .query_one(
                "SELECT lc_precedes($1,$2,$3,$4,$5,$6,$7,$8)",
                &[&a.t.get(), &a.x, &a.y, &a.z, &b.t.get(), &b.x, &b.y, &b.z],
            )
            .await
            .unwrap()
            .get(0)
    }

    /// A client against a store with the schema applied, or `None` where there is no database.
    ///
    /// Skipping rather than failing: the suite has to pass on a machine with no Postgres, and a
    /// test that cannot run is not a test that failed. `LC_STORE_URL` points it elsewhere.
    async fn store() -> Option<Client> {
        let client = crate::connect().await.ok()?;
        apply(&client).await.expect("the schema applies");
        Some(client)
    }

    /// Cheap deterministic noise. Not a distribution, just a spread that does not repeat.
    fn splitmix(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    #[tokio::test]
    async fn the_schema_applies_once_and_then_does_nothing() {
        let Some(client) = store().await else { return };
        // `store` has already applied it, so a second pass must be empty.
        assert!(apply(&client).await.unwrap().is_empty(), "a step ran twice");
        let recorded: i64 = client
            .query_one("SELECT count(*) FROM lc_schema_steps", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(recorded as usize, STEPS.len());
    }

    /// The test that keeps the two implementations of causality honest.
    ///
    /// `lc_precedes` in SQL and `precedes` in `lc-spacetime` are the same predicate written
    /// twice, and a rule built on one has to mean the same thing to the other. Randomised over
    /// the whole coordinate range, with the interesting cases forced in: a `numeric` that
    /// rounded, or a `bigint` difference that overflowed, would show as a disagreement here and
    /// nowhere else until a query silently dropped an event.
    #[tokio::test]
    async fn sql_causality_agrees_with_the_rust_implementation() {
        let Some(client) = store().await else { return };
        let mut seed = 0x1ce4_e5b9_u64;
        let mut pairs: Vec<(Coord, Coord)> = Vec::new();

        // Pairs built to land on the light cone and just off it, where the predicate turns.
        for step in 0..64i64 {
            let separation = 1i64 << (step % 40);
            let a = Coord::new_unchecked(Micros::new(0), 0, 0, 0);
            for offset in [-1, 0, 1] {
                let b = Coord::new_unchecked(
                    Micros::new(separation + offset),
                    separation,
                    0,
                    0,
                );
                pairs.push((a, b));
            }
            // The same, spread over three axes so the sum of squares is what is tested.
            let b = Coord::new_unchecked(Micros::new(separation * 2), separation, separation, separation);
            pairs.push((a, b));
        }

        // And a spread over the whole legal range, including the extremes the bound allows.
        let coordinate = |state: &mut u64| {
            let pick = |state: &mut u64| {
                let raw = splitmix(state) % (COORD_BOUND as u64 * 2);
                raw as i64 - COORD_BOUND
            };
            Coord::new_unchecked(Micros::new(pick(state)), pick(state), pick(state), pick(state))
        };
        for _ in 0..400 {
            pairs.push((coordinate(&mut seed), coordinate(&mut seed)));
        }
        let extreme = COORD_BOUND - 1;
        pairs.push((
            Coord::new_unchecked(Micros::new(-extreme), -extreme, -extreme, -extreme),
            Coord::new_unchecked(Micros::new(extreme), extreme, extreme, extreme),
        ));

        for (a, b) in pairs {
            let row = client
                .query_one(
                    "SELECT lc_precedes($1,$2,$3,$4,$5,$6,$7,$8),
                            lc_classify($1,$2,$3,$4,$5,$6,$7,$8)::text,
                            lc_interval2($1,$2,$3,$4,$5,$6,$7,$8)::text",
                    &[&a.t.get(), &a.x, &a.y, &a.z, &b.t.get(), &b.x, &b.y, &b.z],
                )
                .await
                .unwrap();
            let (sql_precedes, sql_class, sql_interval): (bool, String, String) =
                (row.get(0), row.get(1), row.get(2));

            assert_eq!(
                sql_precedes,
                precedes(a, b),
                "precedes disagreed for {a:?} -> {b:?} (interval {sql_interval})",
            );
            let expected = match classify(a, b) {
                Separation::Timelike => "timelike",
                Separation::Lightlike => "lightlike",
                Separation::Spacelike => "spacelike",
            };
            assert_eq!(sql_class, expected, "classify disagreed for {a:?} -> {b:?}");
            // The interval itself, exactly. A `numeric` that rounded would still classify most
            // pairs correctly and be wrong about the ones that matter.
            assert_eq!(
                sql_interval,
                lc_spacetime::interval::interval2(a, b).to_string(),
                "the interval disagreed for {a:?} -> {b:?}",
            );
        }
    }

    /// The properties a rule is allowed to depend on, checked against the database rather than
    /// against the Rust — the SQL is what a query will actually run.
    #[tokio::test]
    async fn sql_precedence_is_an_order() {
        let Some(client) = store().await else { return };
        let mut seed = 0xbf58_476d_u64;
        let pick = |state: &mut u64| {
            let axis = |state: &mut u64| (splitmix(state) % 2_000_000) as i64 - 1_000_000;
            Coord::new_unchecked(Micros::new(axis(state)), axis(state), axis(state), axis(state))
        };
        for _ in 0..60 {
            let (a, b, c) = (pick(&mut seed), pick(&mut seed), pick(&mut seed));
            assert!(sql_precedes(&client, a, a).await, "reflexive");
            // Antisymmetric: both ways only when they are the same event.
            if a != b && sql_precedes(&client, a, b).await {
                assert!(!sql_precedes(&client, b, a).await, "antisymmetric: {a:?} {b:?}");
            }
            // Transitive.
            if sql_precedes(&client, a, b).await && sql_precedes(&client, b, c).await {
                assert!(sql_precedes(&client, a, c).await, "transitive: {a:?} {b:?} {c:?}");
            }
        }
    }
}
