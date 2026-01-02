//! Database migration system for .em (Exotic Matters) save files.
//!
//! This module provides a versioned migration system for SQLite save files.
//! Each migration has an "up" and "down" SQL statement to move between versions.

use rusqlite::{Connection, Result as SqlResult};

/// A database migration with up/down SQL statements
pub struct Migration {
    /// Human-readable description of what this migration does
    pub description: &'static str,
    /// SQL to apply this migration (move up one version)
    pub up: &'static str,
    /// SQL to revert this migration (move down one version)
    pub down: &'static str,
}

/// All migrations in order. The program version is the length of this array.
/// Each migration moves the database from version N to version N+1.
pub static MIGRATIONS: &[Migration] = &[
    // Version 0 -> 1: Initial schema
    Migration {
        description: "Initial schema - fully normalized database structure",
        up: r#"
            -- ================================================================
            -- Properties table for key-value metadata
            -- ================================================================
            CREATE TABLE IF NOT EXISTS properties (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_properties_key ON properties(key);

            -- ================================================================
            -- Physics settings (singleton table)
            -- ================================================================
            CREATE TABLE IF NOT EXISTS physics (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                gravitational_constant REAL NOT NULL DEFAULT 6.6743015e-11
            );
            INSERT INTO physics (id, gravitational_constant) VALUES (1, 6.6743015e-11);

            -- ================================================================
            -- Simulation time (singleton table)
            -- ================================================================
            CREATE TABLE IF NOT EXISTS sim_time (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                time_julian_days REAL NOT NULL DEFAULT 2451545.0
            );
            INSERT INTO sim_time (id, time_julian_days) VALUES (1, 2451545.0);

            -- ================================================================
            -- View settings (singleton table)
            -- ================================================================
            CREATE TABLE IF NOT EXISTS view_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                distance_scale REAL NOT NULL DEFAULT 1e-9,
                logarithmic_distance_scale INTEGER NOT NULL DEFAULT 0,
                logarithmic_distance_base REAL NOT NULL DEFAULT 10.0,
                body_scale REAL NOT NULL DEFAULT 1e-9,
                logarithmic_body_scale INTEGER NOT NULL DEFAULT 0,
                logarithmic_body_base REAL NOT NULL DEFAULT 10.0,
                show_labels INTEGER NOT NULL DEFAULT 1,
                show_trajectories INTEGER NOT NULL DEFAULT 1,
                trajectory_resolution INTEGER NOT NULL DEFAULT 120
            );
            INSERT INTO view_settings (id) VALUES (1);

            -- ================================================================
            -- Tags for grouping bodies
            -- ================================================================
            CREATE TABLE IF NOT EXISTS tags (
                name TEXT PRIMARY KEY NOT NULL,
                shown INTEGER NOT NULL DEFAULT 0,
                trajectory INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS tag_members (
                tag_name TEXT NOT NULL,
                body_id TEXT NOT NULL,
                PRIMARY KEY (tag_name, body_id),
                FOREIGN KEY (tag_name) REFERENCES tags(name) ON DELETE CASCADE
            );

            -- ================================================================
            -- Bodies - core body information
            -- ================================================================
            CREATE TABLE IF NOT EXISTS bodies (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT,
                mass REAL NOT NULL DEFAULT 0.0,
                major INTEGER NOT NULL DEFAULT 0,
                designation TEXT
            );

            -- ================================================================
            -- Appearances - one row per body, type determines which fields are used
            -- ================================================================
            -- appearance_type: 'Empty', 'DebugBall', 'Star'
            CREATE TABLE IF NOT EXISTS appearances (
                body_id TEXT PRIMARY KEY NOT NULL,
                appearance_type TEXT NOT NULL DEFAULT 'Empty',
                -- DebugBall and Star fields
                radius REAL,
                -- Color (for DebugBall and Star)
                color_r INTEGER,
                color_g INTEGER,
                color_b INTEGER,
                -- Star-specific fields
                light_r INTEGER,
                light_g INTEGER,
                light_b INTEGER,
                absolute_magnitude REAL,
                FOREIGN KEY (body_id) REFERENCES bodies(id) ON DELETE CASCADE
            );

            -- ================================================================
            -- Motives - time-based motive transitions for each body
            -- ================================================================
            -- transition_event: 'Epoch', 'SOIChange', 'Impulse'
            -- motive_type: 'Fixed', 'Newtonian', 'Keplerian'
            CREATE TABLE IF NOT EXISTS motives (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                body_id TEXT NOT NULL,
                time_key INTEGER NOT NULL,
                time_seconds REAL NOT NULL,
                transition_event TEXT NOT NULL,
                motive_type TEXT NOT NULL,
                UNIQUE (body_id, time_key),
                FOREIGN KEY (body_id) REFERENCES bodies(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_motives_body ON motives(body_id);
            CREATE INDEX IF NOT EXISTS idx_motives_time ON motives(time_seconds);

            -- ================================================================
            -- Fixed Motive data
            -- ================================================================
            CREATE TABLE IF NOT EXISTS motive_fixed (
                motive_id INTEGER PRIMARY KEY NOT NULL,
                pos_x REAL NOT NULL,
                pos_y REAL NOT NULL,
                pos_z REAL NOT NULL,
                FOREIGN KEY (motive_id) REFERENCES motives(id) ON DELETE CASCADE
            );

            -- ================================================================
            -- Newtonian Motive data
            -- ================================================================
            CREATE TABLE IF NOT EXISTS motive_newtonian (
                motive_id INTEGER PRIMARY KEY NOT NULL,
                pos_x REAL NOT NULL,
                pos_y REAL NOT NULL,
                pos_z REAL NOT NULL,
                vel_x REAL NOT NULL,
                vel_y REAL NOT NULL,
                vel_z REAL NOT NULL,
                FOREIGN KEY (motive_id) REFERENCES motives(id) ON DELETE CASCADE
            );

            -- ================================================================
            -- Keplerian Motive data
            -- ================================================================
            CREATE TABLE IF NOT EXISTS motive_keplerian (
                motive_id INTEGER PRIMARY KEY NOT NULL,
                primary_id TEXT NOT NULL,
                -- Shape: 'EccentricitySMA' or 'Apsides'
                shape_type TEXT NOT NULL,
                -- EccentricitySMA fields
                eccentricity REAL,
                semi_major_axis REAL,
                -- Apsides fields
                periapsis REAL,
                apoapsis REAL,
                -- Rotation: 'EulerAngles', 'FlatAngles', 'PrecessingEulerAngles'
                rotation_type TEXT NOT NULL,
                -- EulerAngles and PrecessingEulerAngles fields
                inclination REAL,
                longitude_of_ascending_node REAL,
                argument_of_periapsis REAL,
                -- PrecessingEulerAngles additional fields
                apsidal_precession_period REAL,
                nodal_precession_period REAL,
                -- FlatAngles fields
                longitude_of_periapsis REAL,
                -- Epoch: 'MeanAnomaly', 'TimeAtPeriapsisPassage', 'TrueAnomaly', 'J2000'
                epoch_type TEXT NOT NULL,
                -- MeanAnomaly and TrueAnomaly fields
                epoch_julian_day REAL,
                -- MeanAnomaly and J2000 fields
                mean_anomaly REAL,
                -- TrueAnomaly fields
                true_anomaly REAL,
                -- TimeAtPeriapsisPassage fields
                periapsis_time_julian_day REAL,
                FOREIGN KEY (motive_id) REFERENCES motives(id) ON DELETE CASCADE
            );
        "#,
        down: r#"
            DROP TABLE IF EXISTS motive_keplerian;
            DROP TABLE IF EXISTS motive_newtonian;
            DROP TABLE IF EXISTS motive_fixed;
            DROP TABLE IF EXISTS motives;
            DROP TABLE IF EXISTS appearances;
            DROP TABLE IF EXISTS bodies;
            DROP TABLE IF EXISTS tag_members;
            DROP TABLE IF EXISTS tags;
            DROP TABLE IF EXISTS view_settings;
            DROP TABLE IF EXISTS sim_time;
            DROP TABLE IF EXISTS physics;
            DROP TABLE IF EXISTS properties;
        "#,
    },
];

/// Get the current program version (number of migrations available)
pub fn program_version() -> usize {
    MIGRATIONS.len()
}

/// Initialize the properties table if it doesn't exist
pub fn ensure_properties_table(conn: &Connection) -> SqlResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS properties (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        )",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_properties_key ON properties(key)",
        [],
    )?;
    Ok(())
}

/// Get the current database version from properties table
pub fn get_db_version(conn: &Connection) -> SqlResult<usize> {
    let result: Result<String, _> = conn.query_row(
        "SELECT value FROM properties WHERE key = 'version'",
        [],
        |row| row.get(0),
    );

    match result {
        Ok(version_str) => Ok(version_str.parse().unwrap_or(0)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(e) => Err(e),
    }
}

/// Set the database version in properties table
pub fn set_db_version(conn: &Connection, version: usize) -> SqlResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO properties (key, value) VALUES ('version', ?1)",
        [version.to_string()],
    )?;
    Ok(())
}

/// Run all pending migrations to bring the database up to the current program version
pub fn run_migrations(conn: &Connection) -> SqlResult<usize> {
    ensure_properties_table(conn)?;
    
    let db_version = get_db_version(conn)?;
    let target_version = program_version();
    
    if db_version >= target_version {
        return Ok(0); // No migrations needed
    }
    
    let mut migrations_run = 0;
    
    for version in db_version..target_version {
        let migration = &MIGRATIONS[version];
        
        // Run the migration in a transaction
        conn.execute_batch(migration.up)?;
        
        // Update version after successful migration
        set_db_version(conn, version + 1)?;
        migrations_run += 1;
        
        #[cfg(debug_assertions)]
        println!("Migration {}: {}", version + 1, migration.description);
    }
    
    Ok(migrations_run)
}

/// Rollback one migration (for development/testing)
#[allow(dead_code)]
pub fn rollback_migration(conn: &Connection) -> SqlResult<bool> {
    ensure_properties_table(conn)?;
    
    let db_version = get_db_version(conn)?;
    
    if db_version == 0 {
        return Ok(false); // Nothing to rollback
    }
    
    let migration = &MIGRATIONS[db_version - 1];
    
    // Run the down migration
    conn.execute_batch(migration.down)?;
    
    // Update version
    set_db_version(conn, db_version - 1)?;
    
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_migrations_run() {
        let conn = Connection::open_in_memory().unwrap();
        
        let migrations_run = run_migrations(&conn).unwrap();
        assert_eq!(migrations_run, MIGRATIONS.len());
        
        let version = get_db_version(&conn).unwrap();
        assert_eq!(version, program_version());
        
        // Running again should do nothing
        let migrations_run = run_migrations(&conn).unwrap();
        assert_eq!(migrations_run, 0);
    }
    
    #[test]
    fn test_rollback() {
        let conn = Connection::open_in_memory().unwrap();
        
        run_migrations(&conn).unwrap();
        
        let rolled_back = rollback_migration(&conn).unwrap();
        assert!(rolled_back);
        
        let version = get_db_version(&conn).unwrap();
        assert_eq!(version, program_version() - 1);
    }
}
