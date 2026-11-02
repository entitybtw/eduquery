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
    let encoded: String = format!("{filename}-{stamp}.csv")
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'.' { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    let disp = format!("attachment; filename*=UTF-8''{encoded}");
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

/// Число без лишних нулей: 8.5, 100.
fn num(v: Option<f64>) -> String {
    match v {
        Some(x) if (x - x.round()).abs() < 1e-9 => format!("{}", x.round() as i64),
        Some(x) => format!("{x:.2}").trim_end_matches('0').trim_end_matches('.').to_string(),
        None => String::new(),
    }
}

/// «2026-09-14 00:00:00» → «2026-09-14»; со временем — «2026-09-14 10:30».
fn date_out(s: &str) -> String {
    match s.strip_suffix(" 00:00:00") {
        Some(d) => d.to_string(),
        None => s.chars().take(16).collect(),
    }
}

fn header(cols: &[&str]) -> Vec<String> {
    cols.iter().map(|s| s.to_string()).collect()
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
    let mut rows = vec![header(&[
        "ID ученика", "ФИО", "Группа", "Эл. почта", "Курс", "Записей", "Средний балл",
        "Успеваемость, %", "Средняя оценка", "Пропущено уроков", "Из них без уважит. причины",
        "Записей в портфолио", "Поощрений", "Замечаний и нарушений", "Питался, % дней",
        "Время, мин", "Последняя активность",
    ])];
    for s in list {
        rows.push(vec![
            s.external_id,
            s.full_name,
            s.group_name,
            s.email,
            opt(s.grade),
            s.activities.to_string(),
            num(s.avg_score),
            num(s.avg_ratio.map(|v| (v * 1000.0).round() / 10.0)),
            num(s.avg_grade),
            s.absences.to_string(),
            s.absences_unexcused.to_string(),
            s.portfolio.to_string(),
            s.praises.to_string(),
            s.remarks.to_string(),
            if s.meals > 0 { num(Some((s.meals_eaten as f64 / s.meals as f64 * 1000.0).round() / 10.0)) } else { String::new() },
            num(s.total_minutes.map(|v| v.round())),
            s.last_activity.map(|d| date_out(&d)).unwrap_or_default(),
        ]);
    }
    csv_response("ученики", rows)
}

/// Колонки «записей» — те же, что принимает импорт, поэтому файл можно загрузить обратно.
const RECORD_COLUMNS: &[&str] = &[
    "ID ученика", "ФИО", "Группа", "Эл. почта", "Тип записи", "Предмет", "Название", "Дата",
    "Балл", "Макс. балл", "Оценка", "Статус", "Время, мин", "Пропущено уроков",
    "Причина пропуска", "Уважительная", "Категория", "Уровень / оплата", "Комментарий",
];

/// Все записи (работы, оценки, пропуски, портфолио, поведение, питание).
pub async fn export_activities(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ExportQuery>,
) -> Response {
    let mut sql = String::from(
        "SELECT s.external_id, s.full_name, s.group_name, s.email, a.kind, a.subject, a.title,
                a.occurred_at, a.score, a.max_score, a.grade, a.status, a.duration_min, a.amount,
                a.reason, a.category, a.level, a.comment
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
        let kind: String = r.get(4)?;
        let status: String = r.get(11)?;
        let is_absence = kind == "absence";
        Ok(vec![
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            crate::labels::kind_ru(&kind).to_string(),
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            date_out(&r.get::<_, String>(7)?),
            num(r.get::<_, Option<f64>>(8)?),
            num(r.get::<_, Option<f64>>(9)?),
            opt(r.get::<_, Option<i64>>(10)?),
            if is_absence || kind == "portfolio" || kind == "behavior" || kind == "grade" && status == "done" {
                String::new()
            } else {
                crate::labels::status_ru(&status).to_string()
            },
            num(r.get::<_, Option<f64>>(12)?),
            if is_absence { num(Some(r.get::<_, f64>(13)?)) } else { String::new() },
            r.get::<_, String>(14)?,
            if is_absence { crate::labels::yes_no(status == "excused").to_string() } else { String::new() },
            r.get::<_, String>(15)?,
            r.get::<_, String>(16)?,
            r.get::<_, String>(17)?,
        ])
    });
    let mut rows = vec![header(RECORD_COLUMNS)];
    match mapped {
        Ok(it) => rows.extend(it.filter_map(|r| r.ok())),
        Err(e) => return err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
    csv_response("записи", rows)
}

/// Пустой шаблон для импорта с примерами каждого типа записи.
pub async fn import_template() -> Response {
    let r = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let rows = vec![
        header(RECORD_COLUMNS),
        r(&["1001", "Иванов Иван", "7А", "", "Домашняя работа", "Математика", "Дроби, упр. 12", "2026-09-14", "8", "10", "", "выполнено вовремя", "35", "", "", "", "", "", ""]),
        r(&["1001", "Иванов Иван", "7А", "", "Контрольная работа", "Алгебра", "Контрольная №1", "2026-09-20", "17", "20", "", "выполнено вовремя", "40", "", "", "", "", "", ""]),
        r(&["1001", "Иванов Иван", "7А", "", "Оценка", "Математика", "", "2026-09-21", "", "", "5", "", "", "", "", "", "", "", ""]),
        r(&["1001", "Иванов Иван", "7А", "", "Пропуск", "Физика", "", "2026-09-22", "", "", "", "", "", "2", "Болезнь", "да", "", "", "Справка от врача"]),
        r(&["1001", "Иванов Иван", "7А", "", "Пропуск", "", "", "2026-09-25", "", "", "", "", "", "1", "Без уважительной причины", "нет", "", "", ""]),
        r(&["1001", "Иванов Иван", "7А", "", "Портфолио", "", "Олимпиада по математике, призёр", "2026-10-05", "", "", "", "", "", "", "", "", "Грамота", "Региональный", "2 место"]),
        r(&["1001", "Иванов Иван", "7А", "", "Поведение", "", "", "2026-10-07", "", "", "", "", "", "", "", "", "Замечание", "", "Разговаривал на уроке"]),
        r(&["1001", "Иванов Иван", "7А", "", "Поведение", "", "", "2026-10-10", "", "", "", "", "", "", "", "", "Поощрение", "", "Помог с оформлением класса"]),
        r(&["1001", "Иванов Иван", "7А", "", "Питание", "", "", "2026-10-11", "", "", "", "питался", "", "", "", "", "Обед", "Льготное", ""]),
    ];
    csv_response("шаблон-импорта", rows)
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
    let mut rows = vec![header(&["№", "ID ученика", "ФИО", "Модель", "Дата", "Запрос", "Ответ ИИ"])];
    match mapped {
        Ok(it) => rows.extend(it.filter_map(|r| r.ok())),
        Err(e) => return err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
    csv_response("анализы", rows)
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
