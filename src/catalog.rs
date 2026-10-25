use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct AppRecord {
    pub id: String,
    pub name: String,
    pub start_url: String,
    pub icon_path: Option<String>,
}

pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(include_str!("../migrations/001_init.sql"))?;
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN icon_path TEXT", []);
        
        // Clean up any existing duplicates by keeping the oldest row for each start_url.
        // We must manually clean up children tables first to avoid FK constraint failures.
        let duplicates: Vec<String> = {
            let mut stmt = conn.prepare("SELECT id FROM apps WHERE rowid NOT IN (SELECT min(rowid) FROM apps GROUP BY start_url)")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).collect()
        };

        for dup_id in duplicates {
            let _ = conn.execute("DELETE FROM permissions WHERE app_id = ?1", params![dup_id]);
            let _ = conn.execute("DELETE FROM sessions WHERE app_id = ?1", params![dup_id]);
            let _ = conn.execute("DELETE FROM apps WHERE id = ?1", params![dup_id]);
        }

        Ok(Self { conn })
    }

    pub fn add_app(&self, id: &str, name: &str, start_url: &str) -> Result<()> {
        // Prevent duplicates by checking if the start_url already exists
        let mut stmt = self.conn.prepare("SELECT id FROM apps WHERE start_url = ?1")?;
        let mut rows = stmt.query(params![start_url])?;
        if let Some(row) = rows.next()? {
            let existing_id: String = row.get(0)?;
            if existing_id == id {
                return Ok(()); // Already exists with the exact requested id
            }
            // A different id exists for this URL. Delete it so seed_catalog can insert with the expected id.
            let _ = self.conn.execute("DELETE FROM permissions WHERE app_id = ?1", params![existing_id]);
            let _ = self.conn.execute("DELETE FROM sessions WHERE app_id = ?1", params![existing_id]);
            let _ = self.conn.execute("DELETE FROM apps WHERE id = ?1", params![existing_id]);
        }

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        self.conn.execute(
            "INSERT OR REPLACE INTO apps (id, name, start_url, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, name, start_url, now],
        )?;
        Ok(())
    }

    pub fn set_icon_path(&self, app_id: &str, path: &Path) -> Result<()> {
        self.conn.execute(
            "UPDATE apps SET icon_path = ?1 WHERE id = ?2",
            params![path.to_string_lossy(), app_id],
        )?;
        Ok(())
    }

    pub fn set_app_name(&self, app_id: &str, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE apps SET name = ?1 WHERE id = ?2",
            params![name, app_id],
        )?;
        Ok(())
    }

    pub fn list_apps(&self) -> Result<Vec<AppRecord>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, start_url, icon_path FROM apps ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            Ok(AppRecord {
                id: r.get(0)?,
                name: r.get(1)?,
                start_url: r.get(2)?,
                icon_path: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_all_permissions(&self, app_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT capability, scope FROM permissions WHERE app_id = ?1")?;
        let rows = stmt.query_map(params![app_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn grant_permission(&self, app_id: &str, capability: &str, scope: &str) -> Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        self.conn.execute(
            "INSERT OR IGNORE INTO permissions (app_id, capability, scope, granted_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![app_id, capability, scope, now],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn revoke_permission(&self, app_id: &str, capability: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM permissions WHERE app_id = ?1 AND capability = ?2",
            params![app_id, capability],
        )?;
        Ok(())
    }

    pub fn revoke_permission_scope(&self, app_id: &str, capability: &str, scope: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM permissions WHERE app_id = ?1 AND capability = ?2 AND scope = ?3",
            params![app_id, capability, scope],
        )?;
        Ok(())
    }

    pub fn record_session_start(&self, app_id: &str, pid: u32) -> Result<i64> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        self.conn.execute(
            "INSERT INTO sessions (app_id, pid, started_at) VALUES (?1, ?2, ?3)",
            params![app_id, pid, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn end_session(&self, session_id: i64) -> Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        self.conn.execute(
            "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        Ok(())
    }

    pub fn update_session_peak(&self, session_id: i64, peak_bytes: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions
             SET peak_mem_bytes = MAX(COALESCE(peak_mem_bytes, 0), ?1)
             WHERE id = ?2",
            params![peak_bytes as i64, session_id],
        )?;
        Ok(())
    }
}