use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct AppRecord {
    pub id: String,
    pub name: String,
    pub start_url: String,
}

pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(include_str!("../migrations/001_init.sql"))?;
        Ok(Self { conn })
    }

    pub fn add_app(&self, id: &str, name: &str, start_url: &str) -> Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        self.conn.execute(
            "INSERT OR REPLACE INTO apps (id, name, start_url, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, start_url, now],
        )?;
        Ok(())
    }

    pub fn list_apps(&self) -> Result<Vec<AppRecord>> {
        let mut stmt = self.conn.prepare("SELECT id, name, start_url FROM apps ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            Ok(AppRecord {
                id: r.get(0)?,
                name: r.get(1)?,
                start_url: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn record_session_start(&self, app_id: &str, pid: u32) -> Result<i64> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        self.conn.execute(
            "INSERT INTO sessions (app_id, pid, started_at) VALUES (?1, ?2, ?3)",
            params![app_id, pid, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
}