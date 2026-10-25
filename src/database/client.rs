use crate::models::{UpdateStatus, Workload};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Connection, Error, Result, ToSql};

pub fn create_table_if_not_exist() -> Result<()> {
    let conn = Connection::open("data.db")?;
    create_tables(&conn)
}

fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS workloads (
                   id              INTEGER PRIMARY KEY,
                   name            TEXT NOT NULL,
                   image           TEXT NOT NULL,
                   namespace       TEXT NOT NULL,
                   git_ops_repo    TEXT,
                   include_pattern TEXT,
                   exclude_pattern TEXT,
                   update_available TEXT,
                   current_version TEXT NOT NULL,
                   latest_version  TEXT NOT NULL,
                   last_scanned    TEXT NOT NULL,
                   scan_id        INTEGER,
                   scan_type     TEXT,
                   git_directory   TEXT,
                   scan_exhausted TEXT,
                   regex_error TEXT
                   )",
        [],
    )?;
    // Columns added after the initial schema; existing databases get them
    // appended so `SELECT *` column positions stay stable.
    add_column_if_missing(conn, "workloads", "auto_upgrade", "TEXT")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS auto_upgrades (
                   id           INTEGER PRIMARY KEY,
                   name         TEXT NOT NULL,
                   namespace    TEXT NOT NULL,
                   from_version TEXT NOT NULL,
                   to_version   TEXT NOT NULL,
                   status       TEXT NOT NULL,
                   error        TEXT,
                   created_at   TEXT NOT NULL
                   )",
        [],
    )?;
    Ok(())
}

fn add_column_if_missing(conn: &Connection, table: &str, column: &str, sql_type: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let exists = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|c| c.ok())
        .any(|c| c == column);
    if !exists {
        log::info!("Adding column {} to table {}", column, table);
        conn.execute(&format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, sql_type), [])?;
    }
    Ok(())
}

/// Read the optional `auto_upgrade` column; empty strings are stored for
/// workloads without the annotation and map back to `None`.
fn read_auto_upgrade(row: &rusqlite::Row) -> Option<String> {
    row.get::<_, Option<String>>("auto_upgrade")
        .ok()
        .flatten()
        .filter(|p| !p.is_empty())
}

pub fn return_workload(name: String, namespace: String) -> Result<Workload> {
    let conn = Connection::open("data.db")?;
    let mut stmt = conn.prepare("SELECT * FROM workloads WHERE name = ?1 AND namespace = ?2")?;
    let mut workload = stmt.query_map([&name, &namespace], |row| {
        Ok(Workload {
            name: row.get(1)?,
            image: row.get(2)?,
            namespace: row.get(3)?,
            git_ops_repo: row.get(4)?,
            include_pattern: row.get(5)?,
            exclude_pattern: row.get(6)?,
            update_available: row.get(7)?,
            current_version: row.get(8)?,
            latest_version: row.get(9)?,
            last_scanned: row.get(10)?,
            git_directory: row.get(13)?,
            scan_exhausted: row.get(14)?,
            error: row.get(15).ok(),
            auto_upgrade: read_auto_upgrade(row),
        })
    })?;
    if let Some(w) = workload.next() {
        w
    } else {
        Err(rusqlite::Error::QueryReturnedNoRows)
    }
}

pub fn return_all_workloads() -> Result<Vec<Workload>> {
    let conn = Connection::open("data.db")?;
    // only get latest unique name and namespace combinations
    //let mut stmt = conn.prepare("SELECT * FROM workloads WHERE scan_id IN (SELECT MAX(scan_id) FROM workloads GROUP BY scan_type)")?;
    let mut stmt = conn.prepare("WITH MaxScanID AS (
    SELECT
        scan_type,
        MAX(scan_id) AS max_scan_id
    FROM workloads
    GROUP BY scan_type
),
FilteredByScanID AS (
    SELECT
        w.*
    FROM workloads w
    JOIN MaxScanID ms ON w.scan_type = ms.scan_type AND w.scan_id = ms.max_scan_id
),
MaxLastScanned AS (
    SELECT
        name,
        namespace,
        MAX(last_scanned) AS max_last_scanned
    FROM FilteredByScanID
    GROUP BY name, namespace
)
SELECT
    f.*
FROM FilteredByScanID f
JOIN MaxLastScanned mls ON f.name = mls.name AND f.namespace = mls.namespace AND f.last_scanned = mls.max_last_scanned
")?;
    let workloads = stmt.query_map([], |row| {
        Ok(Workload {
            name: row.get(1)?,
            image: row.get(2)?,
            namespace: row.get(3)?,
            git_ops_repo: row.get(4)?,
            include_pattern: row.get(5)?,
            exclude_pattern: row.get(6)?,
            update_available: row.get(7)?,
            current_version: row.get(8)?,
            latest_version: row.get(9)?,
            last_scanned: row.get(10)?,
            git_directory: row.get(13)?,
            scan_exhausted: row.get(14)?,
            error: row.get(15).ok(),
            auto_upgrade: read_auto_upgrade(row),
        })
    })?;
    let mut result = Vec::new();
    for workload in workloads {
        result.push(workload?);
    }
    Ok(result)
}

impl ToSql for UpdateStatus {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.to_string().into())
    }
}

impl FromSql for UpdateStatus {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

pub fn get_latest_scan_id() -> std::result::Result<i32, Error> {
    let conn = Connection::open("data.db")?;
    let mut stmt = conn.prepare("SELECT MAX(scan_id) FROM workloads")?;
    let mut scan_id_iter = stmt.query_map([], |row| row.get(0))?;

    if let Some(scan_id_result) = scan_id_iter.next() {
        return scan_id_result.map(|id: Option<i32>| id.unwrap_or(0)); // Handle potential NULL
    }
    Ok(0)
}

pub fn insert_workload(workload: &Workload, scan_id: i32) -> Result<()> {
    let conn = Connection::open("data.db")?;
    //get scan_id
    conn.execute(
         "INSERT INTO workloads (name, image, namespace, git_ops_repo, include_pattern, exclude_pattern, update_available, current_version, latest_version, last_scanned, scan_id, scan_type, git_directory, scan_exhausted, regex_error, auto_upgrade)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
         [
             &workload.name,
             &workload.image,
             &workload.namespace,
             workload.git_ops_repo.as_deref().unwrap_or_default(),
             workload.include_pattern.as_deref().unwrap_or_default(),
             workload.exclude_pattern.as_deref().unwrap_or_default(),
             &workload.update_available.to_string(),
             &workload.current_version,
             &workload.latest_version,
             &workload.last_scanned,
             &scan_id.to_string(),
             &workload.name,
             workload.git_directory.as_deref().unwrap_or_default(),
             &workload.scan_exhausted.to_string(),
             workload.error.as_deref().unwrap_or(""),
             workload.auto_upgrade.as_deref().unwrap_or_default(),
         ],
     ).map_err(|e| {
         log::error!("Failed to insert workload into database: {}", e);
         e
     }).map(|_| ())
}

pub const AUTO_UPGRADE_SUCCESS: &str = "success";
pub const AUTO_UPGRADE_FAILED: &str = "failed";

/// Record an auto-upgrade attempt for a workload.
pub fn record_auto_upgrade(
    workload: &Workload,
    to_version: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    let conn = Connection::open("data.db")?;
    record_auto_upgrade_with(&conn, workload, to_version, status, error)
}

fn record_auto_upgrade_with(
    conn: &Connection,
    workload: &Workload,
    to_version: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO auto_upgrades (name, namespace, from_version, to_version, status, error, created_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            workload.name,
            workload.namespace,
            workload.current_version,
            to_version,
            status,
            error,
            chrono::Utc::now().to_rfc3339(),
        ],
    )
    .map(|_| ())
}

/// Whether `to_version` was already successfully auto-upgraded for this
/// workload. Used to avoid re-committing the same tag while the GitOps tool
/// has not rolled it out yet, and to respect deliberate rollbacks.
pub fn auto_upgrade_already_applied(name: &str, namespace: &str, to_version: &str) -> Result<bool> {
    let conn = Connection::open("data.db")?;
    auto_upgrade_already_applied_with(&conn, name, namespace, to_version)
}

fn auto_upgrade_already_applied_with(
    conn: &Connection,
    name: &str,
    namespace: &str,
    to_version: &str,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM auto_upgrades
                   WHERE name = ?1 AND namespace = ?2 AND to_version = ?3 AND status = ?4)",
        [name, namespace, to_version, AUTO_UPGRADE_SUCCESS],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workload() -> Workload {
        Workload {
            name: "app".to_string(),
            exclude_pattern: None,
            git_ops_repo: Some("repo".to_string()),
            include_pattern: None,
            update_available: UpdateStatus::Available,
            git_directory: None,
            image: "example/app:1.2.3".to_string(),
            last_scanned: "2026-10-25T00:00:00Z".to_string(),
            namespace: "default".to_string(),
            current_version: "1.2.3".to_string(),
            latest_version: "2.0.0".to_string(),
            scan_exhausted: "False".to_string(),
            error: None,
            auto_upgrade: Some("minor".to_string()),
        }
    }

    #[test]
    fn migrates_old_schema_and_reads_auto_upgrade() {
        let conn = Connection::open_in_memory().unwrap();
        // Schema as created by previous releases (no auto_upgrade column).
        conn.execute(
            "CREATE TABLE workloads (
                   id INTEGER PRIMARY KEY, name TEXT NOT NULL, image TEXT NOT NULL,
                   namespace TEXT NOT NULL, git_ops_repo TEXT, include_pattern TEXT,
                   exclude_pattern TEXT, update_available TEXT, current_version TEXT NOT NULL,
                   latest_version TEXT NOT NULL, last_scanned TEXT NOT NULL, scan_id INTEGER,
                   scan_type TEXT, git_directory TEXT, scan_exhausted TEXT, regex_error TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workloads (name, image, namespace, update_available, current_version,
                   latest_version, last_scanned, scan_id, scan_type, scan_exhausted, regex_error)
                   VALUES ('old', 'img', 'default', 'NotAvailable', '1.0.0', '', 'now', 1, 'old', 'False', '')",
            [],
        )
        .unwrap();

        create_tables(&conn).unwrap();
        // Idempotent on an already-migrated database.
        create_tables(&conn).unwrap();

        conn.execute(
            "INSERT INTO workloads (name, image, namespace, update_available, current_version,
                   latest_version, last_scanned, scan_id, scan_type, scan_exhausted, regex_error, auto_upgrade)
                   VALUES ('new', 'img', 'default', 'NotAvailable', '1.0.0', '', 'now', 2, 'new', 'False', '', 'minor')",
            [],
        )
        .unwrap();

        let mut stmt = conn.prepare("SELECT * FROM workloads ORDER BY id").unwrap();
        let policies: Vec<Option<String>> = stmt
            .query_map([], |row| Ok(read_auto_upgrade(row)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(policies, vec![None, Some("minor".to_string())]);
    }

    #[test]
    fn only_successful_auto_upgrades_are_deduplicated() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        let w = workload();

        assert!(!auto_upgrade_already_applied_with(&conn, "app", "default", "1.3.0").unwrap());

        record_auto_upgrade_with(&conn, &w, "1.3.0", AUTO_UPGRADE_FAILED, Some("boom")).unwrap();
        assert!(!auto_upgrade_already_applied_with(&conn, "app", "default", "1.3.0").unwrap());

        record_auto_upgrade_with(&conn, &w, "1.3.0", AUTO_UPGRADE_SUCCESS, None).unwrap();
        assert!(auto_upgrade_already_applied_with(&conn, "app", "default", "1.3.0").unwrap());
        assert!(!auto_upgrade_already_applied_with(&conn, "app", "default", "1.4.0").unwrap());
        assert!(!auto_upgrade_already_applied_with(&conn, "app", "other", "1.3.0").unwrap());
    }
}
