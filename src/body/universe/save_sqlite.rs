//! SQLite-based save/load system for .em (Exotic Matters) save files.
//!
//! Uses a fully normalized database structure - no JSON blobs.

use std::path::PathBuf;
use std::collections::HashMap;
use bevy::math::DVec3;
use rusqlite::{Connection, Result as SqlResult, params};

use crate::body::appearance::{Appearance, AppearanceColor, DebugBall, StarBall};
use crate::body::motive::info::BodyInfo;
use crate::body::motive::kepler_motive::{
    KeplerMotive, KeplerShape, KeplerRotation, KeplerEpoch,
    EccentricitySMA, Apsides,
    KeplerEulerAngles, KeplerFlatAngles, KeplerPrecessingEulerAngles,
    MeanAnomalyAtEpoch, PeriapsisTime, TrueAnomalyAtEpoch, MeanAnomalyAtJ2000,
};
use crate::body::motive::{Motive, MotiveSelection, TransitionEvent};
use crate::body::universe::save::{
    UniverseFileContents, UniverseFileTime, UniversePhysics, ViewSettings,
    SomeBody, CompoundMotiveEntry,
};
use crate::gui::menu::TagState;
use crate::util::bitfutz;

use super::migrations;

/// Error type for SQLite save operations
#[derive(Debug)]
pub enum SqliteSaveError {
    Sqlite(rusqlite::Error),
    InvalidData(String),
    IO(std::io::Error),
}

impl From<rusqlite::Error> for SqliteSaveError {
    fn from(e: rusqlite::Error) -> Self {
        SqliteSaveError::Sqlite(e)
    }
}

impl From<std::io::Error> for SqliteSaveError {
    fn from(e: std::io::Error) -> Self {
        SqliteSaveError::IO(e)
    }
}

/// Open a connection to an .em file, running migrations as needed
pub fn open_em_file(path: &PathBuf) -> Result<Connection, SqliteSaveError> {
    let conn = Connection::open(path)?;
    
    // Enable foreign keys
    conn.execute("PRAGMA foreign_keys = ON", [])?;
    
    // Run any pending migrations
    migrations::run_migrations(&conn)?;
    
    Ok(conn)
}

/// Create a new .em file with the initial schema
pub fn create_em_file(path: &PathBuf) -> Result<Connection, SqliteSaveError> {
    // Remove existing file if present
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    
    let conn = Connection::open(path)?;
    
    // Enable foreign keys
    conn.execute("PRAGMA foreign_keys = ON", [])?;
    
    // Run all migrations to create schema
    migrations::run_migrations(&conn)?;
    
    Ok(conn)
}

/// Load a UniverseFileContents from an .em file
pub fn load_from_em(path: &PathBuf) -> Result<UniverseFileContents, SqliteSaveError> {
    let conn = open_em_file(path)?;
    
    // Load physics
    let physics = load_physics(&conn)?;
    
    // Load time
    let time = load_time(&conn)?;
    
    // Load view settings
    let view = load_view_settings(&conn)?;
    
    // Load bodies with their motives
    let bodies = load_bodies(&conn)?;
    
    Ok(UniverseFileContents {
        version: format!("em-{}", migrations::program_version()),
        time,
        view,
        physics,
        bodies,
    })
}

/// Save a UniverseFileContents to an .em file
pub fn save_to_em(path: &PathBuf, contents: &UniverseFileContents) -> Result<(), SqliteSaveError> {
    let conn = create_em_file(path)?;
    
    // Save in a transaction
    conn.execute("BEGIN TRANSACTION", [])?;
    
    match (|| -> Result<(), SqliteSaveError> {
        save_physics(&conn, &contents.physics)?;
        save_time(&conn, &contents.time)?;
        // Save bodies first - this creates tags from body.info.tags
        save_bodies(&conn, &contents.bodies)?;
        // Then save view settings - this updates tag display settings (shown/trajectory)
        save_view_settings(&conn, &contents.view)?;
        Ok(())
    })() {
        Ok(()) => {
            conn.execute("COMMIT", [])?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            Err(e)
        }
    }
}

// ============================================================================
// Physics
// ============================================================================

fn load_physics(conn: &Connection) -> Result<UniversePhysics, SqliteSaveError> {
    let gravitational_constant: f64 = conn.query_row(
        "SELECT gravitational_constant FROM physics WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    
    Ok(UniversePhysics { gravitational_constant })
}

fn save_physics(conn: &Connection, physics: &UniversePhysics) -> Result<(), SqliteSaveError> {
    conn.execute(
        "UPDATE physics SET gravitational_constant = ?1 WHERE id = 1",
        [physics.gravitational_constant],
    )?;
    Ok(())
}

// ============================================================================
// Time
// ============================================================================

fn load_time(conn: &Connection) -> Result<UniverseFileTime, SqliteSaveError> {
    let time_julian_days: f64 = conn.query_row(
        "SELECT time_julian_days FROM sim_time WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    
    Ok(UniverseFileTime { time_julian_days })
}

fn save_time(conn: &Connection, time: &UniverseFileTime) -> Result<(), SqliteSaveError> {
    conn.execute(
        "UPDATE sim_time SET time_julian_days = ?1 WHERE id = 1",
        [time.time_julian_days],
    )?;
    Ok(())
}

// ============================================================================
// View Settings
// ============================================================================

fn load_view_settings(conn: &Connection) -> Result<ViewSettings, SqliteSaveError> {
    let row = conn.query_row(
        "SELECT distance_scale, logarithmic_distance_scale, logarithmic_distance_base,
                body_scale, logarithmic_body_scale, logarithmic_body_base,
                show_labels, show_trajectories, trajectory_resolution
         FROM view_settings WHERE id = 1",
        [],
        |row| {
            Ok((
                row.get::<_, f64>(0)?,
                row.get::<_, i32>(1)? != 0,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, i32>(4)? != 0,
                row.get::<_, f64>(5)?,
                row.get::<_, i32>(6)? != 0,
                row.get::<_, i32>(7)? != 0,
                row.get::<_, usize>(8)?,
            ))
        },
    )?;
    
    // Load tags
    let tags = load_tags(conn)?;
    
    Ok(ViewSettings {
        distance_scale: row.0,
        logarithmic_distance_scale: row.1,
        logarithmic_distance_base: row.2,
        body_scale: row.3,
        logarithmic_body_scale: row.4,
        logarithmic_body_base: row.5,
        show_labels: row.6,
        show_trajectories: row.7,
        tags,
        trajectory_resolution: row.8,
    })
}

fn save_view_settings(conn: &Connection, view: &ViewSettings) -> Result<(), SqliteSaveError> {
    conn.execute(
        "UPDATE view_settings SET
            distance_scale = ?1,
            logarithmic_distance_scale = ?2,
            logarithmic_distance_base = ?3,
            body_scale = ?4,
            logarithmic_body_scale = ?5,
            logarithmic_body_base = ?6,
            show_labels = ?7,
            show_trajectories = ?8,
            trajectory_resolution = ?9
         WHERE id = 1",
        params![
            view.distance_scale,
            view.logarithmic_distance_scale as i32,
            view.logarithmic_distance_base,
            view.body_scale,
            view.logarithmic_body_scale as i32,
            view.logarithmic_body_base,
            view.show_labels as i32,
            view.show_trajectories as i32,
            view.trajectory_resolution as i32,
        ],
    )?;
    
    // Save tags
    save_tags(conn, &view.tags)?;
    
    Ok(())
}

// ============================================================================
// Tags
// ============================================================================

fn load_tags(conn: &Connection) -> Result<HashMap<String, TagState>, SqliteSaveError> {
    let mut tags = HashMap::new();
    
    let mut stmt = conn.prepare(
        "SELECT name, shown, trajectory FROM tags"
    )?;
    
    let tag_iter = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i32>(1)? != 0,
            row.get::<_, i32>(2)? != 0,
        ))
    })?;
    
    for tag_result in tag_iter {
        let (name, shown, trajectory) = tag_result?;
        
        // Load members for this tag
        let mut member_stmt = conn.prepare(
            "SELECT body_id FROM tag_members WHERE tag_name = ?1"
        )?;
        let members: Vec<String> = member_stmt
            .query_map([&name], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        
        tags.insert(name, TagState { shown, trajectory, members });
    }
    
    Ok(tags)
}

fn save_tags(conn: &Connection, tags: &HashMap<String, TagState>) -> Result<(), SqliteSaveError> {
    // Update tag display settings (shown/trajectory)
    // Note: Tags and tag_members are already created by save_bodies from body.info.tags
    // This function updates the display settings and can add additional tags from view settings
    
    for (name, state) in tags {
        // Update or insert tag with display settings
        conn.execute(
            "INSERT OR REPLACE INTO tags (name, shown, trajectory) VALUES (?1, ?2, ?3)",
            params![name, state.shown as i32, state.trajectory as i32],
        )?;
        
        // Add any members that might not be in body.info.tags
        for member in &state.members {
            conn.execute(
                "INSERT OR IGNORE INTO tag_members (tag_name, body_id) VALUES (?1, ?2)",
                params![name, member],
            )?;
        }
    }
    
    Ok(())
}

// ============================================================================
// Bodies
// ============================================================================

fn load_bodies(conn: &Connection) -> Result<Vec<SomeBody>, SqliteSaveError> {
    let mut bodies = Vec::new();
    
    let mut stmt = conn.prepare(
        "SELECT id, name, mass, major, designation FROM bodies"
    )?;
    
    let body_iter = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, i32>(3)? != 0,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    
    for body_result in body_iter {
        let (id, name, mass, major, designation) = body_result?;
        
        // Load tags for this body
        let mut tag_stmt = conn.prepare(
            "SELECT tag_name FROM tag_members WHERE body_id = ?1"
        )?;
        let tags: Vec<String> = tag_stmt
            .query_map([&id], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        
        let info = BodyInfo {
            id: id.clone(),
            name,
            mass,
            major,
            designation,
            tags,
        };
        
        // Load appearance
        let appearance = load_appearance(conn, &id)?;
        
        // Load motive
        let motive = load_motive(conn, &id)?;
        
        bodies.push(SomeBody::CompoundMotiveEntry(CompoundMotiveEntry {
            info,
            motive,
            appearance,
        }));
    }
    
    Ok(bodies)
}

fn save_bodies(conn: &Connection, bodies: &[SomeBody]) -> Result<(), SqliteSaveError> {
    for body in bodies {
        let (info, appearance, motive) = match body {
            SomeBody::FixedEntry(e) => {
                let m = Motive::fixed(e.position);
                (&e.info, &e.appearance, m)
            }
            SomeBody::NewtonEntry(e) => {
                let m = Motive::newtonian(e.position, e.velocity);
                (&e.info, &e.appearance, m)
            }
            SomeBody::KeplerEntry(e) => {
                let m = Motive::keplerian(
                    e.params.primary_id.clone(),
                    e.params.shape.clone(),
                    e.params.rotation.clone(),
                    e.params.epoch.clone(),
                );
                (&e.info, &e.appearance, m)
            }
            SomeBody::CompoundEntry(e) => {
                let m = Motive::fixed(DVec3::ZERO);
                (&e.info, &e.appearance, m)
            }
            SomeBody::CompoundMotiveEntry(e) => {
                (&e.info, &e.appearance, e.motive.clone())
            }
        };
        
        // Insert body
        conn.execute(
            "INSERT INTO bodies (id, name, mass, major, designation)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                info.id,
                info.name,
                info.mass,
                info.major as i32,
                info.designation,
            ],
        )?;
        
        // Save body's tags to tag_members
        for tag in &info.tags {
            // Ensure the tag exists in the tags table
            conn.execute(
                "INSERT OR IGNORE INTO tags (name, shown, trajectory) VALUES (?1, 1, 0)",
                [tag],
            )?;
            // Add body as member of this tag
            conn.execute(
                "INSERT OR IGNORE INTO tag_members (tag_name, body_id) VALUES (?1, ?2)",
                params![tag, info.id],
            )?;
        }
        
        // Save appearance
        save_appearance(conn, &info.id, appearance)?;
        
        // Save motive
        save_motive(conn, &info.id, &motive)?;
    }
    
    Ok(())
}

// ============================================================================
// Appearances
// ============================================================================

fn load_appearance(conn: &Connection, body_id: &str) -> Result<Appearance, SqliteSaveError> {
    let result = conn.query_row(
        "SELECT appearance_type, radius, color_r, color_g, color_b,
                light_r, light_g, light_b, absolute_magnitude
         FROM appearances WHERE body_id = ?1",
        [body_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<f64>>(1)?,
                row.get::<_, Option<i32>>(2)?,
                row.get::<_, Option<i32>>(3)?,
                row.get::<_, Option<i32>>(4)?,
                row.get::<_, Option<i32>>(5)?,
                row.get::<_, Option<i32>>(6)?,
                row.get::<_, Option<i32>>(7)?,
                row.get::<_, Option<f32>>(8)?,
            ))
        },
    );
    
    match result {
        Ok((appearance_type, radius, color_r, color_g, color_b, light_r, light_g, light_b, absolute_magnitude)) => {
            match appearance_type.as_str() {
                "Empty" => Ok(Appearance::Empty),
                "DebugBall" => {
                    Ok(Appearance::DebugBall(DebugBall {
                        radius: radius.unwrap_or(1.0),
                        color: AppearanceColor {
                            r: color_r.unwrap_or(255) as u16,
                            g: color_g.unwrap_or(255) as u16,
                            b: color_b.unwrap_or(255) as u16,
                        },
                    }))
                }
                "Star" => {
                    Ok(Appearance::Star(StarBall {
                        radius: radius.unwrap_or(1.0),
                        color: AppearanceColor {
                            r: color_r.unwrap_or(255) as u16,
                            g: color_g.unwrap_or(255) as u16,
                            b: color_b.unwrap_or(255) as u16,
                        },
                        light: AppearanceColor {
                            r: light_r.unwrap_or(255) as u16,
                            g: light_g.unwrap_or(255) as u16,
                            b: light_b.unwrap_or(255) as u16,
                        },
                        absolute_magnitude: absolute_magnitude.unwrap_or(4.83),
                    }))
                }
                _ => Ok(Appearance::Empty),
            }
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Appearance::Empty),
        Err(e) => Err(e.into()),
    }
}

fn save_appearance(conn: &Connection, body_id: &str, appearance: &Appearance) -> Result<(), SqliteSaveError> {
    match appearance {
        Appearance::Empty => {
            conn.execute(
                "INSERT INTO appearances (body_id, appearance_type) VALUES (?1, 'Empty')",
                [body_id],
            )?;
        }
        Appearance::DebugBall(ball) => {
            conn.execute(
                "INSERT INTO appearances (body_id, appearance_type, radius, color_r, color_g, color_b)
                 VALUES (?1, 'DebugBall', ?2, ?3, ?4, ?5)",
                params![
                    body_id,
                    ball.radius,
                    ball.color.r as i32,
                    ball.color.g as i32,
                    ball.color.b as i32,
                ],
            )?;
        }
        Appearance::Star(star) => {
            conn.execute(
                "INSERT INTO appearances (body_id, appearance_type, radius, color_r, color_g, color_b,
                                          light_r, light_g, light_b, absolute_magnitude)
                 VALUES (?1, 'Star', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    body_id,
                    star.radius,
                    star.color.r as i32,
                    star.color.g as i32,
                    star.color.b as i32,
                    star.light.r as i32,
                    star.light.g as i32,
                    star.light.b as i32,
                    star.absolute_magnitude,
                ],
            )?;
        }
    }
    Ok(())
}

// ============================================================================
// Motives
// ============================================================================

fn load_motive(conn: &Connection, body_id: &str) -> Result<Motive, SqliteSaveError> {
    let mut stmt = conn.prepare(
        "SELECT id, time_seconds, transition_event, motive_type
         FROM motives WHERE body_id = ?1 ORDER BY time_seconds"
    )?;
    
    let mut motive = Motive::empty();
    
    let motive_iter = stmt.query_map([body_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, f64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    
    for motive_result in motive_iter {
        let (motive_id, time_seconds, event_str, motive_type) = motive_result?;
        
        let event = parse_transition_event(&event_str)?;
        let selection = load_motive_selection(conn, motive_id, &motive_type)?;
        
        motive.insert_event(time_seconds, event, selection);
    }
    
    // If no motives were loaded, create a default fixed motive
    if motive.is_empty() {
        motive = Motive::fixed(DVec3::ZERO);
    }
    
    Ok(motive)
}

fn load_motive_selection(conn: &Connection, motive_id: i64, motive_type: &str) -> Result<MotiveSelection, SqliteSaveError> {
    match motive_type {
        "Fixed" => {
            let (primary_id, x, y, z): (Option<String>, f64, f64, f64) = conn.query_row(
                "SELECT primary_id, pos_x, pos_y, pos_z FROM motive_fixed WHERE motive_id = ?1",
                [motive_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            Ok(MotiveSelection::Fixed { primary_id, position: DVec3::new(x, y, z) })
        }
        "Newtonian" => {
            let row: (f64, f64, f64, f64, f64, f64) = conn.query_row(
                "SELECT pos_x, pos_y, pos_z, vel_x, vel_y, vel_z FROM motive_newtonian WHERE motive_id = ?1",
                [motive_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )?;
            Ok(MotiveSelection::Newtonian {
                position: DVec3::new(row.0, row.1, row.2),
                velocity: DVec3::new(row.3, row.4, row.5),
            })
        }
        "Keplerian" => {
            let kepler = load_keplerian(conn, motive_id)?;
            Ok(MotiveSelection::Keplerian(kepler))
        }
        _ => Err(SqliteSaveError::InvalidData(format!("Unknown motive type: {}", motive_type))),
    }
}

fn load_keplerian(conn: &Connection, motive_id: i64) -> Result<KeplerMotive, SqliteSaveError> {
    let row = conn.query_row(
        "SELECT primary_id, shape_type, eccentricity, semi_major_axis, periapsis, apoapsis,
                rotation_type, inclination, longitude_of_ascending_node, argument_of_periapsis,
                apsidal_precession_period, nodal_precession_period, longitude_of_periapsis,
                epoch_type, epoch_julian_day, mean_anomaly, true_anomaly, periapsis_time_julian_day
         FROM motive_keplerian WHERE motive_id = ?1",
        [motive_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,   // primary_id
                row.get::<_, String>(1)?,   // shape_type
                row.get::<_, Option<f64>>(2)?,   // eccentricity
                row.get::<_, Option<f64>>(3)?,   // semi_major_axis
                row.get::<_, Option<f64>>(4)?,   // periapsis
                row.get::<_, Option<f64>>(5)?,   // apoapsis
                row.get::<_, String>(6)?,   // rotation_type
                row.get::<_, Option<f64>>(7)?,   // inclination
                row.get::<_, Option<f64>>(8)?,   // longitude_of_ascending_node
                row.get::<_, Option<f64>>(9)?,   // argument_of_periapsis
                row.get::<_, Option<f64>>(10)?,  // apsidal_precession_period
                row.get::<_, Option<f64>>(11)?,  // nodal_precession_period
                row.get::<_, Option<f64>>(12)?,  // longitude_of_periapsis
                row.get::<_, String>(13)?,  // epoch_type
                row.get::<_, Option<f64>>(14)?,  // epoch_julian_day
                row.get::<_, Option<f64>>(15)?,  // mean_anomaly
                row.get::<_, Option<f64>>(16)?,  // true_anomaly
                row.get::<_, Option<f64>>(17)?,  // periapsis_time_julian_day
            ))
        },
    )?;
    
    let (primary_id, shape_type, eccentricity, semi_major_axis, periapsis, apoapsis,
         rotation_type, inclination, longitude_of_ascending_node, argument_of_periapsis,
         apsidal_precession_period, nodal_precession_period, longitude_of_periapsis,
         epoch_type, epoch_julian_day, mean_anomaly, true_anomaly, periapsis_time_julian_day) = row;
    
    // Parse shape
    let shape = match shape_type.as_str() {
        "EccentricitySMA" => KeplerShape::EccentricitySMA(EccentricitySMA {
            eccentricity: eccentricity.unwrap_or(0.0),
            semi_major_axis: semi_major_axis.unwrap_or(1.0),
        }),
        "Apsides" => KeplerShape::Apsides(Apsides {
            periapsis: periapsis.unwrap_or(1.0),
            apoapsis: apoapsis.unwrap_or(2.0),
        }),
        _ => return Err(SqliteSaveError::InvalidData(format!("Unknown shape type: {}", shape_type))),
    };
    
    // Parse rotation
    let rotation = match rotation_type.as_str() {
        "EulerAngles" => KeplerRotation::EulerAngles(KeplerEulerAngles {
            inclination: inclination.unwrap_or(0.0),
            longitude_of_ascending_node: longitude_of_ascending_node.unwrap_or(0.0),
            argument_of_periapsis: argument_of_periapsis.unwrap_or(0.0),
        }),
        "FlatAngles" => KeplerRotation::FlatAngles(KeplerFlatAngles {
            longitude_of_periapsis: longitude_of_periapsis.unwrap_or(0.0),
        }),
        "PrecessingEulerAngles" => KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
            inclination: inclination.unwrap_or(0.0),
            longitude_of_ascending_node: longitude_of_ascending_node.unwrap_or(0.0),
            argument_of_periapsis: argument_of_periapsis.unwrap_or(0.0),
            apsidal_precession_period: apsidal_precession_period.unwrap_or(0.0),
            nodal_precession_period: nodal_precession_period.unwrap_or(0.0),
        }),
        _ => return Err(SqliteSaveError::InvalidData(format!("Unknown rotation type: {}", rotation_type))),
    };
    
    // Parse epoch
    let epoch = match epoch_type.as_str() {
        "MeanAnomaly" => KeplerEpoch::MeanAnomaly(MeanAnomalyAtEpoch {
            epoch_julian_day: epoch_julian_day.unwrap_or(2451545.0),
            mean_anomaly: mean_anomaly.unwrap_or(0.0),
        }),
        "TimeAtPeriapsisPassage" => KeplerEpoch::TimeAtPeriapsisPassage(PeriapsisTime {
            time_julian_day: periapsis_time_julian_day.unwrap_or(2451545.0),
        }),
        "TrueAnomaly" => KeplerEpoch::TrueAnomaly(TrueAnomalyAtEpoch {
            epoch_julian_day: epoch_julian_day.unwrap_or(2451545.0),
            true_anomaly: true_anomaly.unwrap_or(0.0),
        }),
        "J2000" => KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
            mean_anomaly: mean_anomaly.unwrap_or(0.0),
        }),
        _ => return Err(SqliteSaveError::InvalidData(format!("Unknown epoch type: {}", epoch_type))),
    };
    
    Ok(KeplerMotive {
        primary_id,
        shape,
        rotation,
        epoch,
    })
}

fn save_motive(conn: &Connection, body_id: &str, motive: &Motive) -> Result<(), SqliteSaveError> {
    for (time_seconds, event, selection) in motive.iter_events() {
        let time_key = bitfutz::f64::to_u64(time_seconds) as i64;
        let event_str = serialize_transition_event(event);
        let motive_type = match selection {
            MotiveSelection::Fixed { .. } => "Fixed",
            MotiveSelection::Newtonian { .. } => "Newtonian",
            MotiveSelection::Keplerian(_) => "Keplerian",
        };
        
        // Insert motive record
        conn.execute(
            "INSERT INTO motives (body_id, time_key, time_seconds, transition_event, motive_type)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![body_id, time_key, time_seconds, event_str, motive_type],
        )?;
        
        let motive_id = conn.last_insert_rowid();
        
        // Insert type-specific data
        match selection {
            MotiveSelection::Fixed { primary_id, position } => {
                conn.execute(
                    "INSERT INTO motive_fixed (motive_id, primary_id, pos_x, pos_y, pos_z) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![motive_id, primary_id, position.x, position.y, position.z],
                )?;
            }
            MotiveSelection::Newtonian { position, velocity } => {
                conn.execute(
                    "INSERT INTO motive_newtonian (motive_id, pos_x, pos_y, pos_z, vel_x, vel_y, vel_z)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![motive_id, position.x, position.y, position.z, velocity.x, velocity.y, velocity.z],
                )?;
            }
            MotiveSelection::Keplerian(kepler) => {
                save_keplerian(conn, motive_id, kepler)?;
            }
        }
    }
    
    Ok(())
}

fn save_keplerian(conn: &Connection, motive_id: i64, kepler: &KeplerMotive) -> Result<(), SqliteSaveError> {
    // Extract shape data
    let (shape_type, eccentricity, semi_major_axis, periapsis, apoapsis) = match &kepler.shape {
        KeplerShape::EccentricitySMA(esma) => (
            "EccentricitySMA",
            Some(esma.eccentricity),
            Some(esma.semi_major_axis),
            None,
            None,
        ),
        KeplerShape::Apsides(aps) => (
            "Apsides",
            None,
            None,
            Some(aps.periapsis),
            Some(aps.apoapsis),
        ),
    };
    
    // Extract rotation data
    let (rotation_type, inclination, longitude_of_ascending_node, argument_of_periapsis,
         apsidal_precession_period, nodal_precession_period, longitude_of_periapsis_val) = match &kepler.rotation {
        KeplerRotation::EulerAngles(ea) => (
            "EulerAngles",
            Some(ea.inclination),
            Some(ea.longitude_of_ascending_node),
            Some(ea.argument_of_periapsis),
            None,
            None,
            None,
        ),
        KeplerRotation::FlatAngles(fa) => (
            "FlatAngles",
            None,
            None,
            None,
            None,
            None,
            Some(fa.longitude_of_periapsis),
        ),
        KeplerRotation::PrecessingEulerAngles(pea) => (
            "PrecessingEulerAngles",
            Some(pea.inclination),
            Some(pea.longitude_of_ascending_node),
            Some(pea.argument_of_periapsis),
            Some(pea.apsidal_precession_period),
            Some(pea.nodal_precession_period),
            None,
        ),
    };
    
    // Extract epoch data
    let (epoch_type, epoch_julian_day, mean_anomaly, true_anomaly_val, periapsis_time_julian_day) = match &kepler.epoch {
        KeplerEpoch::MeanAnomaly(ma) => (
            "MeanAnomaly",
            Some(ma.epoch_julian_day),
            Some(ma.mean_anomaly),
            None,
            None,
        ),
        KeplerEpoch::TimeAtPeriapsisPassage(tpp) => (
            "TimeAtPeriapsisPassage",
            None,
            None,
            None,
            Some(tpp.time_julian_day),
        ),
        KeplerEpoch::TrueAnomaly(ta) => (
            "TrueAnomaly",
            Some(ta.epoch_julian_day),
            None,
            Some(ta.true_anomaly),
            None,
        ),
        KeplerEpoch::J2000(j) => (
            "J2000",
            None,
            Some(j.mean_anomaly),
            None,
            None,
        ),
    };
    
    conn.execute(
        "INSERT INTO motive_keplerian (
            motive_id, primary_id,
            shape_type, eccentricity, semi_major_axis, periapsis, apoapsis,
            rotation_type, inclination, longitude_of_ascending_node, argument_of_periapsis,
            apsidal_precession_period, nodal_precession_period, longitude_of_periapsis,
            epoch_type, epoch_julian_day, mean_anomaly, true_anomaly, periapsis_time_julian_day
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            motive_id,
            kepler.primary_id,
            shape_type,
            eccentricity,
            semi_major_axis,
            periapsis,
            apoapsis,
            rotation_type,
            inclination,
            longitude_of_ascending_node,
            argument_of_periapsis,
            apsidal_precession_period,
            nodal_precession_period,
            longitude_of_periapsis_val,
            epoch_type,
            epoch_julian_day,
            mean_anomaly,
            true_anomaly_val,
            periapsis_time_julian_day,
        ],
    )?;
    
    Ok(())
}

fn parse_transition_event(s: &str) -> Result<TransitionEvent, SqliteSaveError> {
    match s {
        "Epoch" => Ok(TransitionEvent::Epoch),
        "SOIChange" => Ok(TransitionEvent::SOIChange),
        "Impulse" => Ok(TransitionEvent::Impulse),
        _ => Err(SqliteSaveError::InvalidData(format!("Unknown transition event: {}", s))),
    }
}

fn serialize_transition_event(event: &TransitionEvent) -> &'static str {
    match event {
        TransitionEvent::Epoch => "Epoch",
        TransitionEvent::SOIChange => "SOIChange",
        TransitionEvent::Impulse => "Impulse",
    }
}
