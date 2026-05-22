use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

pub const DATABASE_FILENAME: &str = "codegraph.db";
pub const CURRENT_SCHEMA_VERSION: i64 = 4;

pub struct DatabaseInfo {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub schema_version: i64,
}

pub fn get_database_path(project_root: &Path) -> PathBuf {
    project_root.join(".codegraph").join(DATABASE_FILENAME)
}

pub fn initialize_database(project_root: &Path) -> Result<DatabaseInfo> {
    let db_path = get_database_path(project_root);
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open {}", db_path.display()))?;
    configure_database(&conn)?;
    conn.execute_batch(include_str!("../../src/db/schema.sql"))
        .context("failed to initialize schema")?;

    let current_version = get_current_version(&conn)?;
    if current_version < CURRENT_SCHEMA_VERSION {
        conn.execute(
            "INSERT OR IGNORE INTO schema_versions (version, applied_at, description) VALUES (?1, ?2, ?3)",
            params![
                CURRENT_SCHEMA_VERSION,
                unix_time_ms(),
                "Initial schema includes all migrations"
            ],
        )?;
    }

    database_info(&db_path, &conn)
}

pub fn open_database(project_root: &Path) -> Result<DatabaseInfo> {
    let db_path = get_database_path(project_root);
    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open {}", db_path.display()))?;
    configure_database(&conn)?;
    database_info(&db_path, &conn)
}

pub fn open_connection(project_root: &Path) -> Result<Connection> {
    let db_path = get_database_path(project_root);
    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open {}", db_path.display()))?;
    configure_database(&conn)?;
    Ok(conn)
}

fn configure_database(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 120_000)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "cache_size", -64_000)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 268_435_456)?;
    Ok(())
}

fn get_current_version(conn: &Connection) -> Result<i64> {
    let version = conn
        .query_row("SELECT MAX(version) FROM schema_versions", [], |row| row.get::<_, Option<i64>>(0))
        .optional()?
        .flatten()
        .unwrap_or(0);
    Ok(version)
}

fn database_info(db_path: &Path, conn: &Connection) -> Result<DatabaseInfo> {
    let schema_version = get_current_version(conn)?;
    let size_bytes = fs::metadata(db_path)
        .with_context(|| format!("failed to stat {}", db_path.display()))?
        .len();

    Ok(DatabaseInfo {
        path: db_path.to_path_buf(),
        size_bytes,
        schema_version,
    })
}

fn unix_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
