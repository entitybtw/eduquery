//! Пользователи: хранение в SQLite, права доступа и лимиты запросов к ИИ.

use anyhow::{anyhow, bail, Result};
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

use crate::db::Db;

/// Все существующие права.
pub const ALL_PERMS: &[&str] = &["view", "analyze", "import", "export", "models", "users"];
/// Права по умолчанию для нового пользователя.
pub const DEFAULT_PERMS: &[&str] = &["view", "analyze", "export"];

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: String,
    /// Эффективные права (у администратора — все).
    pub perms: Vec<String>,
    pub daily_limit: i64,
    pub hourly_limit: i64,
    pub active: bool,
    pub created_at: String,
    pub last_login: Option<String>,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    pub fn has(&self, perm: &str) -> bool {
        self.is_admin() || self.perms.iter().any(|p| p == perm)
    }

    /// Виртуальный администратор для режима без авторизации.
    pub fn anonymous() -> Self {
        Self {
            id: 0,
            username: "anonymous".into(),
            role: "admin".into(),
            perms: ALL_PERMS.iter().map(|s| s.to_string()).collect(),
            daily_limit: 0,
            hourly_limit: 0,
            active: true,
            created_at: String::new(),
            last_login: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UserWithUsage {
    #[serde(flatten)]
    pub user: User,
    pub used_hour: i64,
    pub used_day: i64,
}

pub struct NewUser {
    pub username: String,
    pub password: String,
    pub role: String,
    pub perms: Vec<String>,
    pub daily_limit: i64,
    pub hourly_limit: i64,
}

#[derive(Default)]
pub struct UserPatch {
    pub role: Option<String>,
    pub perms: Option<Vec<String>>,
    pub daily_limit: Option<i64>,
    pub hourly_limit: Option<i64>,
    pub active: Option<bool>,
    pub password: Option<String>,
}

const COLS: &str =
    "id, username, role, perms, daily_limit, hourly_limit, active, created_at, last_login";

fn row_to_user(r: &Row) -> rusqlite::Result<User> {
    let role: String = r.get(2)?;
    let perms_raw: String = r.get(3)?;
    let perms = if role == "admin" {
        ALL_PERMS.iter().map(|s| s.to_string()).collect()
    } else {
        perms_raw
            .split(',')
            .map(str::trim)
            .filter(|p| ALL_PERMS.contains(p))
            .map(String::from)
            .collect()
    };
    Ok(User {
        id: r.get(0)?,
        username: r.get(1)?,
        role,
        perms,
        daily_limit: r.get(4)?,
        hourly_limit: r.get(5)?,
        active: r.get::<_, i64>(6)? != 0,
        created_at: r.get(7)?,
        last_login: r.get(8)?,
    })
}

// ─────────────────────────────── пароли ───────────────────────────────

fn hash_password(pw: &str) -> Result<String> {
    use rand::RngCore;
    let mut raw = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut raw);
    let salt = SaltString::encode_b64(&raw).map_err(|e| anyhow!("salt: {e}"))?;
    Ok(Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map_err(|e| anyhow!("hash: {e}"))?
        .to_string())
}

fn verify_password(pw: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(pw.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

// ─────────────────────────────── валидация ───────────────────────────────

fn valid_username(u: &str) -> bool {
    (2..=32).contains(&u.chars().count())
        && u.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '@'))
}

fn valid_password(p: &str) -> Result<()> {
    if p.chars().count() < 6 {
        bail!("пароль должен быть не короче 6 символов");
    }
    Ok(())
}

fn clean_role(r: &str) -> Result<String> {
    match r {
        "admin" | "user" => Ok(r.to_string()),
        _ => bail!("роль должна быть admin или user"),
    }
}

fn clean_perms(p: &[String]) -> String {
    ALL_PERMS
        .iter()
        .filter(|a| p.iter().any(|x| x == *a))
        .cloned()
        .collect::<Vec<_>>()
        .join(",")
}

// ─────────────────────────────── операции ───────────────────────────────

/// Создать первого администратора из ENV, если пользователей ещё нет.
pub fn ensure_admin(db: &Db, username: &str, password: &str) -> Result<()> {
    let n: i64 = db.conn().query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    if n > 0 {
        return Ok(());
    }
    let hash = hash_password(password)?;
    db.conn().execute(
        "INSERT INTO users(username, password_hash, role, perms) VALUES (?1, ?2, 'admin', ?3)",
        params![username, hash, ALL_PERMS.join(",")],
    )?;
    tracing::info!("создан администратор «{username}»");
    Ok(())
}

pub fn create(db: &Db, nu: NewUser) -> Result<i64> {
    let username = nu.username.trim().to_string();
    if !valid_username(&username) {
        bail!("логин: 2–32 символа, буквы, цифры и _ . - @");
    }
    valid_password(&nu.password)?;
    let role = clean_role(&nu.role)?;
    let hash = hash_password(&nu.password)?;
    let conn = db.conn();
    let exists: bool = conn
        .query_row("SELECT 1 FROM users WHERE username = ?1", [&username], |_| Ok(()))
        .optional()?
        .is_some();
    if exists {
        bail!("пользователь «{username}» уже существует");
    }
    conn.execute(
        "INSERT INTO users(username, password_hash, role, perms, daily_limit, hourly_limit)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            username,
            hash,
            role,
            clean_perms(&nu.perms),
            nu.daily_limit.max(0),
            nu.hourly_limit.max(0)
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_by_name(db: &Db, username: &str) -> Option<User> {
    db.conn()
        .query_row(
            &format!("SELECT {COLS} FROM users WHERE username = ?1"),
            [username],
            row_to_user,
        )
        .optional()
        .ok()
        .flatten()
}

pub fn get_by_id(db: &Db, id: i64) -> Option<User> {
    db.conn()
        .query_row(&format!("SELECT {COLS} FROM users WHERE id = ?1"), [id], row_to_user)
        .optional()
        .ok()
        .flatten()
}

pub fn list(db: &Db) -> Result<Vec<UserWithUsage>> {
    let users: Vec<User> = {
        let conn = db.conn();
        let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM users ORDER BY id"))?;
        let rows = stmt.query_map([], row_to_user)?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    Ok(users
        .into_iter()
        .map(|u| {
            let (h, d) = usage(db, u.id);
            UserWithUsage { user: u, used_hour: h, used_day: d }
        })
        .collect())
}

fn active_admins(db: &Db) -> i64 {
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM users WHERE role = 'admin' AND active = 1",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
}

pub fn update(db: &Db, id: i64, patch: UserPatch) -> Result<()> {
    let current = get_by_id(db, id).ok_or_else(|| anyhow!("пользователь не найден"))?;

    // Не оставляем систему без активного администратора.
    let loses_admin = current.is_admin()
        && current.active
        && (patch.role.as_deref().map(|r| r != "admin").unwrap_or(false)
            || patch.active == Some(false));
    if loses_admin && active_admins(db) <= 1 {
        bail!("нельзя убрать последнего активного администратора");
    }

    let conn = db.conn();
    if let Some(role) = &patch.role {
        conn.execute("UPDATE users SET role = ?2 WHERE id = ?1", params![id, clean_role(role)?])?;
    }
    if let Some(p) = &patch.perms {
        conn.execute("UPDATE users SET perms = ?2 WHERE id = ?1", params![id, clean_perms(p)])?;
    }
    if let Some(v) = patch.daily_limit {
        conn.execute("UPDATE users SET daily_limit = ?2 WHERE id = ?1", params![id, v.max(0)])?;
    }
    if let Some(v) = patch.hourly_limit {
        conn.execute("UPDATE users SET hourly_limit = ?2 WHERE id = ?1", params![id, v.max(0)])?;
    }
    if let Some(a) = patch.active {
        conn.execute("UPDATE users SET active = ?2 WHERE id = ?1", params![id, a as i64])?;
    }
    if let Some(pw) = &patch.password {
        valid_password(pw)?;
        conn.execute(
            "UPDATE users SET password_hash = ?2 WHERE id = ?1",
            params![id, hash_password(pw)?],
        )?;
    }
    Ok(())
}

pub fn delete(db: &Db, id: i64) -> Result<()> {
    let u = get_by_id(db, id).ok_or_else(|| anyhow!("пользователь не найден"))?;
    if u.is_admin() && u.active && active_admins(db) <= 1 {
        bail!("нельзя удалить последнего активного администратора");
    }
    let conn = db.conn();
    conn.execute("DELETE FROM usage_log WHERE user_id = ?1", [id])?;
    conn.execute("DELETE FROM sessions WHERE username = ?1", [&u.username])?;
    conn.execute("DELETE FROM users WHERE id = ?1", [id])?;
    Ok(())
}

/// Проверка логина и пароля. Ошибка — текст для пользователя.
pub fn authenticate(db: &Db, username: &str, password: &str) -> std::result::Result<User, String> {
    let row: Option<(i64, String)> = db
        .conn()
        .query_row(
            "SELECT id, password_hash FROM users WHERE username = ?1",
            [username.trim()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    let Some((id, hash)) = row else {
        return Err("неверный логин или пароль".into());
    };
    if !verify_password(password, &hash) {
        return Err("неверный логин или пароль".into());
    }
    let user = get_by_id(db, id).ok_or("неверный логин или пароль")?;
    if !user.active {
        return Err("учётная запись отключена".into());
    }
    let _ = db
        .conn()
        .execute("UPDATE users SET last_login = datetime('now') WHERE id = ?1", [id]);
    Ok(user)
}

/// Проверить старый пароль и поставить новый.
pub fn change_password(db: &Db, user: &User, old: &str, new: &str) -> Result<()> {
    authenticate(db, &user.username, old).map_err(|_| anyhow!("текущий пароль неверен"))?;
    valid_password(new)?;
    update(db, user.id, UserPatch { password: Some(new.to_string()), ..Default::default() })
}

// ─────────────────────────────── лимиты ───────────────────────────────

/// Сколько запросов к ИИ было за последний час и за 24 часа.
pub fn usage(db: &Db, user_id: i64) -> (i64, i64) {
    let conn = db.conn();
    let count = |window: &str| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM usage_log WHERE user_id = ?1 AND at >= datetime('now', ?2)",
            params![user_id, window],
            |r| r.get(0),
        )
        .unwrap_or(0)
    };
    (count("-1 hour"), count("-1 day"))
}

/// Проверить лимиты и, если всё в порядке, засчитать запрос.
pub fn check_and_record(db: &Db, user: &User) -> std::result::Result<(), String> {
    if user.id == 0 {
        return Ok(());
    }
    let (h, d) = usage(db, user.id);
    if user.hourly_limit > 0 && h >= user.hourly_limit {
        return Err(format!(
            "лимит запросов к ИИ исчерпан: {} в час. Попробуйте позже.",
            user.hourly_limit
        ));
    }
    if user.daily_limit > 0 && d >= user.daily_limit {
        return Err(format!(
            "лимит запросов к ИИ исчерпан: {} за 24 часа. Попробуйте позже.",
            user.daily_limit
        ));
    }
    let conn = db.conn();
    let _ = conn.execute("INSERT INTO usage_log(user_id) VALUES (?1)", [user.id]);
    // Чистим журнал старше 30 дней.
    let _ = conn.execute("DELETE FROM usage_log WHERE at < datetime('now', '-30 day')", []);
    Ok(())
}
