//! Аутентификация: логин по логину/паролю, сессии в cookie и Bearer-токене.

use anyhow::Result;
use rusqlite::params;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use crate::db::Db;

/// Сессия пользователя.
#[derive(Debug, Clone)]
pub struct Session {
    pub username: String,
    pub expires_at: SystemTime,
}

/// Менеджер сессий. Учётные записи живут в таблице `users`.
pub struct Auth {
    allow_anonymous: bool,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    ttl: Duration,
}

impl Auth {
    pub fn new(allow_anonymous: bool) -> Self {
        Self {
            allow_anonymous,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            ttl: Duration::from_secs(60 * 60 * 24),
        }
    }

    /// Проверить логин/пароль и выдать токен.
    pub fn login(&self, db: &Db, user: &str, pass: &str) -> Result<String, String> {
        let u = crate::users::authenticate(db, user, pass)?;
        let token = gen_token();
        let session = Session {
            username: u.username,
            expires_at: SystemTime::now() + self.ttl,
        };
        let mut s = self.sessions.lock().unwrap();
        s.retain(|_, v| v.expires_at > SystemTime::now());
        s.insert(token.clone(), session);
        Ok(token)
    }

    pub fn logout(&self, token: &str) {
        self.sessions.lock().unwrap().remove(token);
    }

    /// Проверить токен; возвращает логин владельца сессии.
    pub fn validate(&self, token: &str) -> Option<String> {
        let mut s = self.sessions.lock().unwrap();
        match s.get(token) {
            Some(sess) if sess.expires_at > SystemTime::now() => Some(sess.username.clone()),
            Some(_) => {
                s.remove(token);
                None
            }
            None => None,
        }
    }

    pub fn require_auth(&self) -> bool {
        !self.allow_anonymous
    }
}

/// Восстановить сессии из БД (чтобы не терять вход при рестарте).
pub fn restore_sessions(db: &Db, auth: &Auth) -> Result<()> {
    let conn = db.conn();
    let mut stmt = conn.prepare("SELECT token, username, expires_at FROM sessions")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
    })?;
    let mut sessions = auth.sessions.lock().unwrap();
    for row in rows {
        let (token, username, expires) = row?;
        if let Ok(exp) = chrono::NaiveDateTime::parse_from_str(&expires, "%Y-%m-%d %H:%M:%S") {
            let exp_utc = exp.and_utc();
            let now = chrono::Utc::now();
            if exp_utc > now {
                sessions.insert(
                    token.clone(),
                    Session {
                        username,
                        expires_at: SystemTime::now()
                            + Duration::from_secs((exp_utc - now).num_seconds().max(0) as u64),
                    },
                );
            }
        }
    }
    Ok(())
}

/// Сохранить сессию в БД.
pub fn persist_session(db: &Db, token: &str, username: &str, expires_at: SystemTime) -> Result<()> {
    let exp = chrono::DateTime::<chrono::Utc>::from(expires_at)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    db.conn().execute(
        "INSERT OR REPLACE INTO sessions(token, username, expires_at) VALUES (?1, ?2, ?3)",
        params![token, username, exp],
    )?;
    Ok(())
}

/// Удалить сессию из БД.
pub fn drop_session(db: &Db, token: &str) -> Result<()> {
    db.conn()
        .execute("DELETE FROM sessions WHERE token = ?1", params![token])?;
    Ok(())
}

fn gen_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
