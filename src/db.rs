//! Слой доступа к данным: SQLite-схема, миграции и seed-данные.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// Обёртка над SQLite-соединением.
///
/// `Connection` в rusqlite не является `Sync`, поэтому держим его под
/// мьютексом — БД у нас одна, а операции быстрые.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("не удалось открыть БД {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    /// Доступ к соединению (блокировка берётся на время использования).
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn migrate(&self) -> Result<()> {
        self.conn().execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS students (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                external_id  TEXT NOT NULL UNIQUE,
                full_name    TEXT NOT NULL,
                group_name   TEXT NOT NULL DEFAULT '',
                email        TEXT NOT NULL DEFAULT '',
                grade        INTEGER,
                created_at   TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS activities (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                student_id   INTEGER NOT NULL REFERENCES students(id) ON DELETE CASCADE,
                kind         TEXT NOT NULL,           -- task | quiz | submission | attendance | grade | note
                title        TEXT NOT NULL DEFAULT '',
                subject      TEXT NOT NULL DEFAULT '',
                score        REAL,
                max_score    REAL,
                grade        INTEGER,
                status       TEXT NOT NULL DEFAULT 'done',
                duration_min REAL,
                occurred_at  TEXT NOT NULL,
                details      TEXT NOT NULL DEFAULT '{}',
                created_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_act_student ON activities(student_id);
            CREATE INDEX IF NOT EXISTS idx_act_time    ON activities(occurred_at);

            CREATE TABLE IF NOT EXISTS models (
                id           TEXT PRIMARY KEY,
                name         TEXT NOT NULL,
                file_path    TEXT NOT NULL,
                size_bytes   INTEGER NOT NULL DEFAULT 0,
                sha256       TEXT NOT NULL DEFAULT '',
                params_json  TEXT NOT NULL DEFAULT '{}',
                source       TEXT NOT NULL DEFAULT 'custom',
                created_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS analyses (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                student_id   INTEGER NOT NULL REFERENCES students(id) ON DELETE CASCADE,
                model_id     TEXT NOT NULL DEFAULT '',
                prompt       TEXT NOT NULL DEFAULT '',
                answer       TEXT NOT NULL DEFAULT '',
                created_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_an_student ON analyses(student_id);

            CREATE TABLE IF NOT EXISTS sessions (
                token      TEXT PRIMARY KEY,
                username   TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                expires_at TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Option<String> {
        self.conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }
}
