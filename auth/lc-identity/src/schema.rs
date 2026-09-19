//! The migrations, and the one function that applies them.
//!
//! **This crate owns the schema.** The broker applies it at boot from the copy that shipped in
//! its image, so the schema and the code that assumes it are one artifact — which is the only
//! way a rollback rolls back both. The administration console reads and writes the same tables
//! and deliberately does not run these: two services migrating one database is two advisory
//! locks and a race between whichever container starts first.
//!
//! It lives in the library rather than in `main.rs` so that a test can build the real schema
//! rather than a hand-written approximation of it. An approximation is a second definition,
//! and the tests that run against one pass for a shape nothing in production has.

/// Apply every migration. sqlx takes a PostgreSQL advisory lock, so this is safe with more
/// than one replica starting at once.
pub async fn apply(pool: &sqlx::PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
