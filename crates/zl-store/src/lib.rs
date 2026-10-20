use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use zl_catalog::Project;
use zl_crawler::CrawlReport;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("stored data: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Install {
    pub project_id: String,
    pub version: Option<String>,
    pub filename: String,
    pub method: String,
    pub handle: serde_json::Value,
    pub installed_at: i64,
}

pub struct Store {
    db: Mutex<Connection>,
    read: Option<Mutex<Connection>>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS catalogue (id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL, fetched_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS crawl (project_id TEXT PRIMARY KEY, json TEXT NOT NULL, crawled_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS installs (
    project_id TEXT PRIMARY KEY,
    version TEXT,
    filename TEXT NOT NULL,
    method TEXT NOT NULL,
    handle TEXT NOT NULL,
    installed_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, json TEXT NOT NULL);
";

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let db = Self::tune(Connection::open(path)?)?;
        db.execute_batch(SCHEMA)?;
        let read = Self::tune(Connection::open(path)?)?;
        Ok(Self { db: Mutex::new(db), read: Some(Mutex::new(read)) })
    }

    pub fn in_memory() -> Result<Self> {
        let db = Connection::open_in_memory()?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db: Mutex::new(db), read: None })
    }

    fn tune(conn: Connection) -> Result<Connection> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        Ok(conn)
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn reader(&self) -> std::sync::MutexGuard<'_, Connection> {
        match &self.read {
            Some(r) => r.lock().unwrap_or_else(|e| e.into_inner()),
            None => self.conn(),
        }
    }

    pub fn save_catalogue(&self, projects: &[Project]) -> Result<()> {
        let json = serde_json::to_string(projects)?;
        self.conn().execute(
            "INSERT INTO catalogue (id, json, fetched_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json, fetched_at = excluded.fetched_at",
            params![json, now()],
        )?;
        Ok(())
    }

    pub fn catalogue(&self) -> Result<Option<(Vec<Project>, i64)>> {
        let row: Option<(String, i64)> = self
            .reader()
            .query_row("SELECT json, fetched_at FROM catalogue WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(match row {
            Some((json, at)) => Some((serde_json::from_str(&json)?, at)),
            None => None,
        })
    }

    pub fn save_crawl(&self, report: &CrawlReport) -> Result<()> {
        self.conn().execute(
            "INSERT INTO crawl (project_id, json, crawled_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(project_id) DO UPDATE SET json = excluded.json, crawled_at = excluded.crawled_at",
            params![report.project_id, serde_json::to_string(report)?, now()],
        )?;
        Ok(())
    }

    pub fn crawls(&self) -> Result<Vec<(CrawlReport, i64)>> {
        let conn = self.reader();
        let mut st = conn.prepare("SELECT json, crawled_at FROM crawl")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (json, at) = row?;
            out.push((serde_json::from_str(&json)?, at));
        }
        Ok(out)
    }

    pub fn crawl(&self, project_id: &str) -> Result<Option<(CrawlReport, i64)>> {
        let row: Option<(String, i64)> = self
            .reader()
            .query_row("SELECT json, crawled_at FROM crawl WHERE project_id = ?1", [project_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        Ok(match row {
            Some((json, at)) => Some((serde_json::from_str(&json)?, at)),
            None => None,
        })
    }

    pub fn clear_crawls(&self) -> Result<()> {
        self.conn().execute("DELETE FROM crawl", [])?;
        Ok(())
    }

    pub fn save_install(&self, i: &Install) -> Result<()> {
        self.conn().execute(
            "INSERT INTO installs (project_id, version, filename, method, handle, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(project_id) DO UPDATE SET version = excluded.version, filename = excluded.filename,
               method = excluded.method, handle = excluded.handle, installed_at = excluded.installed_at",
            params![i.project_id, i.version, i.filename, i.method, serde_json::to_string(&i.handle)?, i.installed_at],
        )?;
        Ok(())
    }

    pub fn installs(&self) -> Result<Vec<Install>> {
        let conn = self.reader();
        let mut st = conn.prepare(
            "SELECT project_id, version, filename, method, handle, installed_at FROM installs ORDER BY project_id",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (project_id, version, filename, method, handle, installed_at) = row?;
            out.push(Install { project_id, version, filename, method, handle: serde_json::from_str(&handle)?, installed_at });
        }
        Ok(out)
    }

    pub fn install(&self, project_id: &str) -> Result<Option<Install>> {
        Ok(self.installs()?.into_iter().find(|i| i.project_id == project_id))
    }

    pub fn remove_install(&self, project_id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM installs WHERE project_id = ?1", [project_id])?;
        Ok(())
    }

    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let json: Option<String> = self
            .reader()
            .query_row("SELECT json FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (key, json) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET json = excluded.json",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_see_writes_across_connections() {
        let dir = std::env::temp_dir().join(format!("zl-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&dir.join("t.db")).unwrap();
        assert!(s.read.is_some());
        s.set_setting("k", &1).unwrap();
        assert_eq!(s.setting::<i32>("k").unwrap(), Some(1));
        s.set_setting("k", &2).unwrap();
        assert_eq!(s.setting::<i32>("k").unwrap(), Some(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trips() {
        let s = Store::in_memory().unwrap();
        assert!(s.catalogue().unwrap().is_none());
        s.save_catalogue(&[]).unwrap();
        assert!(s.catalogue().unwrap().unwrap().0.is_empty());

        let r = CrawlReport { project_id: "zpad".into(), version: Some("0.1.0".into()), ..Default::default() };
        s.save_crawl(&r).unwrap();
        s.save_crawl(&r).unwrap();
        assert_eq!(s.crawls().unwrap().len(), 1);
        assert_eq!(s.crawl("zpad").unwrap().unwrap().0.version.as_deref(), Some("0.1.0"));

        let i = Install {
            project_id: "zpad".into(),
            version: Some("0.1.0".into()),
            filename: "ZPad_0.1.0_amd64.deb".into(),
            method: "pkgbuild".into(),
            handle: serde_json::json!({"package": "zpad"}),
            installed_at: 1,
        };
        s.save_install(&i).unwrap();
        assert_eq!(s.install("zpad").unwrap(), Some(i));
        s.remove_install("zpad").unwrap();
        assert!(s.installs().unwrap().is_empty());

        s.set_setting("theme", &"dark").unwrap();
        assert_eq!(s.setting::<String>("theme").unwrap().as_deref(), Some("dark"));
    }
}
