//! The Lightcone event store.
//!
//! Continuous state is a function and only its parameters change; those changes are the events.
//! This crate holds them, and answers the one question the game's core loop asks — what can this
//! observer see by now — without scanning them. See `lightcone/docs/02-event-store.md`.
//!
//! No engine and no game rules: a schema, the causality predicate, and a cursor over both.

#![forbid(unsafe_code)]

pub mod migrate;

/// Where the store lives, from the environment or the development default.
///
/// `LC_STORE_URL` overrides it. The default is a local socket connection to a database called
/// `lc_store`, which is what a developer gets from `createdb lc_store` and nothing else.
pub fn connection_string() -> String {
    if let Ok(url) = std::env::var("LC_STORE_URL") {
        return url;
    }
    // The operating-system user, the way `psql` defaults it. Naming one here would be naming
    // whichever developer wrote this line.
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME"));
    match user {
        Ok(user) => format!("host=/tmp user={user} dbname=lc_store"),
        Err(_) => "host=/tmp dbname=lc_store".to_string(),
    }
}

/// Connect, and drive the connection in the background.
///
/// `tokio_postgres` hands back a client and a connection future that has to be polled for the
/// client to do anything; spawning it here is what lets a caller treat the client as a plain
/// handle.
pub async fn connect() -> Result<tokio_postgres::Client, tokio_postgres::Error> {
    let (client, connection) =
        tokio_postgres::connect(&connection_string(), tokio_postgres::NoTls).await?;
    tokio::spawn(async move {
        // Nothing to do about a dropped connection here; the next query reports it.
        let _ = connection.await;
    });
    Ok(client)
}
