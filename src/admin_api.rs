//! API управления: пользователи, экспорт таблиц, установка движка.

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::server::AppState;
use crate::students;
use crate::users::{self, NewUser, User, UserPatch};

fn err(msg: impl AsRef<str>, code: StatusCode) -> Response {
    (code, Json(json!({"error": msg.as_ref()}))).into_response()
}

// ─────────────────────────────── пользователи ───────────────────────────────

pub async fn list_users(State(state): State<Arc<AppState>>) -> Response {
    match users::list(&state.db) {
        Ok(list) => Json(json!({"users": list, "all_perms": users::ALL_PERMS})).into_response(),
        Err(e) => err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Deserialize)]
pub struct CreateUserReq {
    username: String,
    password: String,
    #[serde(default = "default_role")]
    role: String,
    #[serde(default)]
    perms: Option<Vec<String>>,
    #[serde(default)]
    daily_limit: i64,
    #[serde(default)]
    hourly_limit: i64,
}

fn default_role() -> String {
    "user".into()
}

pub async fn create_user(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateUserReq>,
) -> Response {
    let perms = req
        .perms
        .unwrap_or_else(|| users::DEFAULT_PERMS.iter().map(|s| s.to_string()).collect());
    let nu = NewUser {
        username: req.username,
        password: req.password,
        role: req.role,
        perms,
        daily_limit: req.daily_limit,
        hourly_limit: req.hourly_limit,
    };
    match users::create(&state.db, nu) {
        Ok(id) => Json(json!({"status": "ok", "id": id})).into_response(),
        Err(e) => err(e.to_string(), StatusCode::BAD_REQUEST),
    }
}

#[derive(Deserialize)]
pub struct UpdateUserReq {
    role: Option<String>,
    perms: Option<Vec<String>>,
    daily_limit: Option<i64>,
    hourly_limit: Option<i64>,
    active: Option<bool>,
    password: Option<String>,
}

pub async fn update_user(
    State(state): State<Arc<AppState>>,
    Extension(me): Extension<User>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateUserReq>,
) -> Response {
    if id == me.id && (req.active == Some(false) || req.role.as_deref().map(|r| r != "admin").unwrap_or(false)) {
        return err("нельзя отключить или понизить самого себя", StatusCode::BAD_REQUEST);
    }
    let patch = UserPatch {
        role: req.role,
        perms: req.perms,
        daily_limit: req.daily_limit,
        hourly_limit: req.hourly_limit,
        active: req.active,
        password: req.password.filter(|p| !p.is_empty()),
    };
    match users::update(&state.db, id, patch) {
        Ok(()) => Json(json!({"status": "ok"})).into_response(),
        Err(e) => err(e.to_string(), StatusCode::BAD_REQUEST),
    }
}

pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    Extension(me): Extension<User>,
    Path(id): Path<i64>,
) -> Response {
    if id == me.id {
        return err("нельзя удалить самого себя", StatusCode::BAD_REQUEST);
    }
    match users::delete(&state.db, id) {
        Ok(()) => Json(json!({"status": "ok"})).into_response(),
        Err(e) => err(e.to_string(), StatusCode::BAD_REQUEST),
    }
}

#[derive(Deserialize)]
pub struct PasswordReq {
    old_password: String,
    new_password: String,
}

pub async fn change_my_password(
    State(state): State<Arc<AppState>>,
    Extension(me): Extension<User>,
    Json(req): Json<PasswordReq>,
) -> Response {
    if me.id == 0 {
        return err("в анонимном режиме пароля нет", StatusCode::BAD_REQUEST);
    }
    match users::change_password(&state.db, &me, &req.old_password, &req.new_password) {
        Ok(()) => Json(json!({"status": "ok"})).into_response(),
        Err(e) => err(e.to_string(), StatusCode::BAD_REQUEST),
    }
}

// ─────────────────────────────── экспорт CSV ───────────────────────────────

fn csv_response(filename: &str, rows: Vec<Vec<String>>) -> Response {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    for r in &rows {
        let _ = wtr.write_record(r);
    }
    let body = wtr.into_inner().unwrap_or_default();
    // BOM, чтобы Excel правильно открыл UTF-8.
    let mut out = vec![0xEF, 0xBB, 0xBF];
    out.extend_from_slice(&body);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M");
    let disp = format!("attachment; filename=\"{filename}-{stamp}.csv\"");
    let mut resp = (StatusCode::OK, out).into_response();
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    if let Ok(v) = HeaderValue::from_str(&disp) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    resp
}

fn opt<T: ToString>(v: Option<T>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

#[derive(Deserialize)]
pub struct ExportQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    group: String,
    #[serde(default)]
    sort: String,
}

/// Текущая таблица учеников (с учётом поиска/фильтра/сортировки).
pub async fn export_students(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExportQuery>,
) -> Response {
    let list = match students::list_students(&state.db, &q.q, &q.group, &q.sort) {
        Ok(l) => l,
        Err(e) => return err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    };
    let mut rows = vec![[
        "external_id", "full_name", "group", "email", "grade", "activities",
        "avg_score", "avg_ratio_percent", "total_minutes", "last_activity",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect::<Vec<_>>()];
    for s in list {
        rows.push(vec![
            s.external_id,
            s.full_name,
            s.group_name,
            s.email,
            opt(s.grade),
            s.activities.to_string(),
            opt(s.avg_score.map(|v| format!("{v:.2}"))),
            opt(s.avg_ratio.map(|v| format!("{:.1}", v * 100.0))),
            opt(s.total_minutes.map(|v| format!("{v:.0}"))),
            opt(s.last_activity),
        ]);
    }
    csv_response("students", rows)
}

/// Все активности в формате, который принимает импорт (можно загрузить обратно).
pub async fn export_activities(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExportQuery>,
) -> Response {
    let mut sql = String::from(
        "SELECT s.external_id, s.full_name, s.group_name, s.email, a.subject, a.title, a.kind,
                a.score, a.max_score, a.grade, a.status, a.duration_min, a.occurred_at
         FROM activities a JOIN students s ON s.id = a.student_id WHERE 1=1",
    );
    let mut args: Vec<String> = Vec::new();
    if !q.q.is_empty() {
        sql.push_str(" AND (s.full_name LIKE ? OR s.external_id LIKE ? OR s.group_name LIKE ?)");
        let like = format!("%{}%", q.q);
        args.extend([like.clone(), like.clone(), like]);
    }
    if !q.group.is_empty() {
        sql.push_str(" AND s.group_name = ?");
        args.push(q.group.clone());
    }
    sql.push_str(" ORDER BY s.id, a.occurred_at");

    let conn = state.db.conn();
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => return err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    };
    let mapped = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| {
        Ok(vec![
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            opt(r.get::<_, Option<f64>>(7)?),
            opt(r.get::<_, Option<f64>>(8)?),
            opt(r.get::<_, Option<i64>>(9)?),
            r.get::<_, String>(10)?,
            opt(r.get::<_, Option<f64>>(11)?),
            r.get::<_, String>(12)?,
        ])
    });
    let mut rows = vec![[
        "external_id", "full_name", "group", "email", "subject", "title", "kind",
        "score", "max_score", "grade", "status", "duration_min", "date",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect::<Vec<_>>()];
    match mapped {
        Ok(it) => rows.extend(it.filter_map(|r| r.ok())),
        Err(e) => return err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
    csv_response("activities", rows)
}

/// История ИИ-анализов.
pub async fn export_analyses(State(state): State<Arc<AppState>>) -> Response {
    let conn = state.db.conn();
    let mut stmt = match conn.prepare(
        "SELECT a.id, s.external_id, s.full_name, a.model_id, a.created_at, a.prompt, a.answer
         FROM analyses a JOIN students s ON s.id = a.student_id ORDER BY a.id",
    ) {
        Ok(s) => s,
        Err(e) => return err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    };
    let mapped = stmt.query_map([], |r| {
        Ok(vec![
            r.get::<_, i64>(0)?.to_string(),
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
        ])
    });
    let mut rows = vec![["id", "external_id", "full_name", "model", "created_at", "prompt", "answer"]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()];
    match mapped {
        Ok(it) => rows.extend(it.filter_map(|r| r.ok())),
        Err(e) => return err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
    csv_response("analyses", rows)
}

// ─────────────────────────────── движок llama.cpp ───────────────────────────────

pub async fn engine_status(State(state): State<Arc<AppState>>) -> Response {
    Json(state.engine.install_status()).into_response()
}

pub async fn engine_install(State(state): State<Arc<AppState>>) -> Response {
    if state.engine.installed() {
        return Json(json!({"status": "already_installed"})).into_response();
    }
    match state.engine.start_install() {
        Ok(()) => Json(json!({"status": "started"})).into_response(),
        Err(e) => err(e, StatusCode::CONFLICT),
    }
}
