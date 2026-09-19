//! The migrations, and the one function that applies them.
//!
//! **This crate owns the schema.** The console reads and writes the same tables and does not
//! run these: two services migrating one database race on first start.
//!
//! In the library rather than in `main.rs` so a test builds the real schema instead of an
//! approximation that passes for a shape nothing in production has.

/// sqlx takes a PostgreSQL advisory lock, so this is safe with several replicas starting.
pub async fn apply(pool: &sqlx::PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
