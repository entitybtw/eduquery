//! HTTP-сервер: REST API + отдача веб-интерфейса.

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::analysis;
use crate::admin_api;
use crate::auth::{self, Auth};
use crate::users::{self, User};
use crate::db::Db;
use crate::llm::{ChatMessage, Engine, GenParams};
use crate::models::{self, ModelRecord, ModelStore};
use crate::students;

/// Общее состояние приложения.
pub struct AppState {
    pub db: Db,
    pub engine: Arc<Engine>,
    pub store: Arc<ModelStore>,
    pub auth: Arc<Auth>,
    pub web_dir: PathBuf,
    pub allow_remote_download: bool,
}

/// Точка входа.
pub async fn serve(state: Arc<AppState>, addr: SocketAddr) -> anyhow::Result<()> {
    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("eduquery слушает на http://{addr}");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn build_router(state: Arc<AppState>) -> Router {
    // Роутер с состоянием `Arc<AppState>` — иначе хендлеры не смогут
    // извлечь `State<Arc<AppState>>`.
    let api: Router<Arc<AppState>> = Router::new()
        // ── auth ──
        .route("/api/v1/login", post(login))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/me", get(me))
        .route("/api/v1/me/password", post(admin_api::change_my_password))
        // ── пользователи ──
        .route("/api/v1/users", get(admin_api::list_users).post(admin_api::create_user))
        .route(
            "/api/v1/users/{id}",
            put(admin_api::update_user).delete(admin_api::delete_user),
        )
        // ── экспорт ──
        .route("/api/v1/export/students.csv", get(admin_api::export_students))
        .route("/api/v1/export/activities.csv", get(admin_api::export_activities))
        .route("/api/v1/export/analyses.csv", get(admin_api::export_analyses))
        // ── движок llama.cpp ──
        .route("/api/v1/engine/status", get(admin_api::engine_status))
        .route("/api/v1/engine/install", post(admin_api::engine_install))
        // ── обзор ──
        .route("/api/v1/overview", get(overview))
        .route("/api/v1/groups", get(groups))
        // ── ученики ──
        .route("/api/v1/students", get(list_students))
        .route("/api/v1/students/{id}", get(get_student))
        .route("/api/v1/students/{id}/context", get(student_context))
        .route("/api/v1/students/{id}/records", post(add_record))
        .route("/api/v1/records/{id}", delete(delete_record))
        .route("/api/v1/stats", get(stats))
        .route("/api/v1/import/template.csv", get(admin_api::import_template))
        .route("/api/v1/seed", post(seed_demo))
        .route("/api/v1/import", post(import_data))
        // ── модели ──
        .route("/api/v1/models/catalog", get(catalog))
        .route("/api/v1/models", get(list_models))
        .route("/api/v1/models/download", post(download_model))
        .route("/api/v1/models/download/custom", post(download_custom))
        .route("/api/v1/models/activate", post(activate_model))
        .route("/api/v1/models/{id}", delete(delete_model))
        .route("/api/v1/models/status", get(models_status))
        // ── генерация ──
        .route("/api/v1/generate", post(generate))
        .route("/api/v1/analyze/student", post(analyze_student))
        .route("/api/v1/analyze/compare", post(analyze_compare))
        .route("/api/v1/analyze/overview", post(analyze_overview))
        .route("/api/v1/analyze/ask", post(analyze_ask))
        .route("/api/v1/params", get(get_params).post(save_params))
        .route("/api/v1/settings/{key}", get(get_setting).post(set_setting))
        .route("/health", get(health));

    let web = state.web_dir.clone();
    let router = api
        // Слой авторизации — на все API-роуты.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        // Статика отдаётся без авторизации.
        .fallback_service(ServeDir::new(web).append_index_html_on_directories(true))
        .fallback_service(ServeDir::new("web"))
        .layer(TraceLayer::new_for_http())
        // Браузер всегда перепроверяет файлы: после обновления сразу видна новая версия.
        .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
        .with_state(state);
    router
}

// ─────────────────────────────── Middleware ───────────────────────────────

/// Извлечь токен из cookie `eq_token` или заголовка Authorization.
fn extract_token(req: &axum::http::Request<axum::body::Body>) -> Option<String> {
    if let Some(cookie) = req.headers().get(header::COOKIE) {
        if let Ok(s) = cookie.to_str() {
            for part in s.split(';') {
                let part = part.trim();
                if let Some(v) = part.strip_prefix("eq_token=") {
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    if let Some(h) = req.headers().get(header::AUTHORIZATION) {
        if let Ok(s) = h.to_str() {
            if let Some(v) = s.strip_prefix("Bearer ") {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Токен сессии, положенный middleware в extensions.
#[derive(Clone)]
pub struct Token(pub String);

/// Какое право нужно для маршрута.
enum Need {
    Any,
    Perm(&'static str),
}

fn required_perm(method: &axum::http::Method, path: &str) -> Need {
    use axum::http::Method;
    let read = method == Method::GET;
    match path {
        "/api/v1/logout" | "/api/v1/me" | "/api/v1/me/password" | "/api/v1/engine/status" => Need::Any,
        "/api/v1/engine/install" => Need::Perm("models"),
        "/api/v1/import" | "/api/v1/seed" | "/api/v1/import/template.csv" => Need::Perm("import"),
        p if p.starts_with("/api/v1/records/") => Need::Perm("import"),
        p if p.starts_with("/api/v1/students/") && method == Method::POST => Need::Perm("import"),
        "/api/v1/generate" => Need::Perm("analyze"),
        "/api/v1/models/catalog" | "/api/v1/models/status" | "/api/v1/models" if read => Need::Any,
        "/api/v1/params" if read => Need::Any,
        "/api/v1/params" => Need::Perm("models"),
        p if p.starts_with("/api/v1/users") => Need::Perm("users"),
        p if p.starts_with("/api/v1/export/") => Need::Perm("export"),
        p if p.starts_with("/api/v1/analyze/") => Need::Perm("analyze"),
        p if p.starts_with("/api/v1/models") => Need::Perm("models"),
        p if p.starts_with("/api/v1/settings/") && read => Need::Any,
        p if p.starts_with("/api/v1/settings/") => Need::Perm("users"),
        _ => Need::Perm("view"),
    }
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    // Публичные маршруты.
    if path == "/health"
        || path == "/api/v1/login"
        || path.starts_with("/assets/")
        || path == "/favicon.ico"
    {
        return next.run(req).await;
    }

    let (user, token) = if !state.auth.require_auth() {
        (User::anonymous(), String::new())
    } else {
        let Some(token) = extract_token(&req) else {
            return json_err("требуется авторизация", StatusCode::UNAUTHORIZED);
        };
        let Some(username) = state.auth.validate(&token) else {
            return json_err("сессия истекла, войдите заново", StatusCode::UNAUTHORIZED);
        };
        match users::get_by_name(&state.db, &username) {
            Some(u) if u.active => (u, token),
            _ => return json_err("учётная запись недоступна", StatusCode::UNAUTHORIZED),
        }
    };

    if let Need::Perm(p) = required_perm(&method, &path) {
        if !user.has(p) {
            return json_err(
                &format!("недостаточно прав (нужно право «{p}»)"),
                StatusCode::FORBIDDEN,
            );
        }
        // Запросы к ИИ учитываются в лимитах пользователя.
        if p == "analyze" && method == axum::http::Method::POST {
            if let Err(msg) = users::check_and_record(&state.db, &user) {
                return json_err(&msg, StatusCode::TOO_MANY_REQUESTS);
            }
        }
    }

    req.extensions_mut().insert(Token(token));
    req.extensions_mut().insert(user);
    next.run(req).await
}

/// Долгие запросы к ИИ: пока модель думает, шлём пробелы-«пульс» каждые 10 секунд,
/// чтобы обратный прокси (nginx и др.) не обрывал соединение по таймауту (504).
/// JSON допускает ведущие пробелы, поэтому клиент получает обычный ответ.
/// Статус при этом всегда 200 — ошибка генерации приходит как `{"error": ...}`.
fn keepalive<F>(fut: F) -> Response
where
    F: std::future::Future<Output = Response> + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel::<axum::body::Bytes>(8);
    tokio::spawn(async move {
        tokio::pin!(fut);
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
        tick.tick().await;
        let resp = loop {
            tokio::select! {
                r = &mut fut => break r,
                _ = tick.tick() => { let _ = tx.send(axum::body::Bytes::from_static(b" ")).await; }
            }
        };
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap_or_default();
        let _ = tx.send(body).await;
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|b| (Ok::<_, std::convert::Infallible>(b), rx))
    });
    let mut resp = Response::new(axum::body::Body::from_stream(stream));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    resp
}

// ──────────────────────────────── Handlers ────────────────────────────────

fn json_err(msg: &str, code: StatusCode) -> Response {
    (code, Json(json!({"error": msg}))).into_response()
}

async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginReq>,
) -> Response {
    match state.auth.login(&state.db, &req.username, &req.password) {
        Ok(token) => {
            // Сохраняем сессию, чтобы пережить рестарт.
            let exp = std::time::SystemTime::now() + std::time::Duration::from_secs(60 * 60 * 24);
            let _ = auth::persist_session(&state.db, &token, &req.username, exp);

            let mut cookie = format!("eq_token={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400");
            if !state.auth.require_auth() {
                cookie = format!("eq_token={token}; Path=/; SameSite=Lax; Max-Age=86400");
            }
            let mut headers = axum::http::HeaderMap::new();
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                headers.insert(header::SET_COOKIE, v);
            }
            (headers, Json(json!({"status": "ok", "token": token}))).into_response()
        }
        Err(msg) => (StatusCode::UNAUTHORIZED, Json(json!({"error": msg}))).into_response(),
    }
}

async fn logout(State(state): State<Arc<AppState>>, Extension(Token(token)): Extension<Token>) -> Response {
    state.auth.logout(&token);
    let _ = auth::drop_session(&state.db, &token);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_static("eq_token=; Path=/; HttpOnly; Max-Age=0"),
    );
    (headers, Json(json!({"status": "ok"}))).into_response()
}

async fn me(
    State(state): State<Arc<AppState>>,
    Extension(Token(token)): Extension<Token>,
    Extension(user): Extension<User>,
) -> Response {
    let (used_hour, used_day) = users::usage(&state.db, user.id);
    Json(json!({
        "anonymous": !state.auth.require_auth(),
        "user": user.username,
        "account": user,
        "usage": {"hour": used_hour, "day": used_day},
        "token": token,
        "active_model": state.engine.active_model(),
        "loaded_models": state.engine.loaded_models(),
        "engine_installed": state.engine.installed(),
    }))
    .into_response()
}

// ── обзор ──

async fn overview(State(state): State<Arc<AppState>>) -> Response {
    match students::overview(&state.db) {
        Ok(o) => Json(o).into_response(),
        Err(e) => {
            tracing::error!("overview: {e}");
            json_err(&format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn groups(State(state): State<Arc<AppState>>) -> Response {
    match students::list_groups(&state.db) {
        Ok(g) => Json(json!({"groups": g})).into_response(),
        Err(e) => json_err(&format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// ── ученики ──

#[derive(Deserialize)]
struct StudentQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    group: String,
    #[serde(default)]
    sort: String,
}

async fn list_students(
    State(state): State<Arc<AppState>>,
    Query(q): Query<StudentQuery>,
) -> Response {
    match students::list_students(&state.db, &q.q, &q.group, &q.sort) {
        Ok(list) => Json(json!({"students": list, "count": list.len()})).into_response(),
        Err(e) => {
            tracing::error!("list_students: {e}");
            json_err(&format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn get_student(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> Response {
    match students::get_student(&state.db, id) {
        Ok(Some(d)) => {
            let profile = analysis::build_profile_json(&d);
            Json(json!({
                "summary": d.summary,
                "by_subject": d.by_subject,
                "by_kind": d.by_kind,
                "timeline": d.timeline,
                "recent": d.recent,
                "strengths": d.strengths,
                "weaknesses": d.weaknesses,
                "grades": d.grades,
                "grade_items": d.grade_items,
                "absence": d.absence,
                "portfolio_items": d.portfolio_items,
                "behavior": d.behavior,
                "meals": d.meals,
                "profile": profile,
            }))
            .into_response()
        }
        Ok(None) => json_err("ученик не найден", StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::error!("get_student: {e}");
            json_err(&format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn student_context(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> Response {
    match analysis::build_student_context(&state.db, id) {
        Ok(ctx) => Json(json!({"context": ctx})).into_response(),
        Err(e) => json_err(&format!("не удалось собрать контекст: {e}"), StatusCode::BAD_REQUEST),
    }
}

async fn seed_demo(State(state): State<Arc<AppState>>) -> Response {
    match crate::seed::seed(&state.db) {
        Ok(n) => Json(json!({"status": "ok", "activities": n})).into_response(),
        Err(e) => {
            tracing::error!("seed: {e}");
            json_err(&format!("ошибка генерации демо-данных: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[derive(Deserialize)]
struct ImportQuery {
    /// "replace" — удалить все текущие данные перед импортом.
    #[serde(default)]
    mode: String,
}

/// Импорт: JSON-массив, CSV-файл или сырой CSV-текст.
async fn import_data(
    State(state): State<Arc<AppState>>,
    Query(iq): Query<ImportQuery>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // UTF-8, а если не получилось — Windows-1251 (Excel в русской локали).
    let text = match String::from_utf8(body.to_vec()) {
        Ok(t) => t,
        Err(_) => encoding_rs::WINDOWS_1251.decode(&body).0.into_owned(),
    };

    if iq.mode == "replace" {
        if let Err(e) = students::clear_all(&state.db) {
            return json_err(&format!("не удалось очистить данные: {e}"), StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    // JSON-массив объектов.
    if ct.contains("json") || text.trim_start().starts_with('[') {
        return match students::import_json(&state.db, &text) {
            Ok(r) => Json(r).into_response(),
            Err(e) => json_err(&format!("ошибка импорта: {e}"), StatusCode::BAD_REQUEST),
        };
    }

    match students::import_csv(&state.db, &text) {
        Ok(r) => Json(r).into_response(),
        Err(e) => {
            tracing::error!("import: {e}");
            json_err(&format!("ошибка импорта: {e}"), StatusCode::BAD_REQUEST)
        }
    }
}

/// Добавить запись (оценка, пропуск, портфолио, поведение…) ученику.
async fn add_record(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(row): Json<students::ImportRow>,
) -> Response {
    match students::add_record(&state.db, id, &row) {
        Ok(()) => Json(json!({"status": "ok"})).into_response(),
        Err(e) => json_err(&format!("не удалось добавить запись: {e}"), StatusCode::BAD_REQUEST),
    }
}

async fn delete_record(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> Response {
    match students::delete_record(&state.db, id) {
        Ok(true) => Json(json!({"status": "ok"})).into_response(),
        Ok(false) => json_err("запись не найдена", StatusCode::NOT_FOUND),
        Err(e) => json_err(&format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Deserialize)]
struct StatsQuery {
    #[serde(default)]
    group: String,
}

/// Данные для вкладки «Статистика» (с фильтром по группе).
async fn stats(State(state): State<Arc<AppState>>, Query(q): Query<StatsQuery>) -> Response {
    match students::stats(&state.db, &q.group) {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            tracing::error!("stats: {e}");
            json_err(&format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ── модели ──

async fn catalog() -> Response {
    Json(json!({"catalog": models::CATALOG})).into_response()
}

async fn list_models(State(state): State<Arc<AppState>>) -> Response {
    let conn = state.db.conn();
    let mut stmt = match conn.prepare(
        "SELECT id, name, file_path, size_bytes, sha256, params_json, source, created_at
         FROM models ORDER BY created_at DESC",
    ) {
        Ok(s) => s,
        Err(e) => return json_err(&format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    };
    let rows = stmt.query_map([], |r| {
        Ok(ModelRecord {
            id: r.get(0)?,
            name: r.get(1)?,
            file_path: r.get(2)?,
            size_bytes: r.get(3)?,
            sha256: r.get(4)?,
            params_json: r.get(5)?,
            source: r.get(6)?,
            created_at: r.get(7)?,
        })
    });
    let list: Vec<ModelRecord> = match rows {
        Ok(it) => it.filter_map(|r| r.ok()).collect(),
        Err(e) => return json_err(&format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    };

    let files = state.store.list_files();
    Json(json!({
        "models": list,
        "files": files,
        "active": state.engine.active_model(),
        "loaded": state.engine.loaded_models(),
        "downloads": state.store.downloads(),
    }))
    .into_response()
}

async fn models_status(State(state): State<Arc<AppState>>) -> Response {
    Json(json!({
        "downloads": state.store.downloads(),
        "active": state.engine.active_model(),
        "loaded": state.engine.loaded_models(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct DownloadReq {
    id: String,
}

async fn download_model(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DownloadReq>,
) -> Response {
    if !state.allow_remote_download {
        return json_err("загрузка из сети отключена", StatusCode::FORBIDDEN);
    }
    let Some(entry) = models::catalog_entry(&req.id) else {
        return json_err("модель не найдена в каталоге", StatusCode::NOT_FOUND);
    };
    spawn_download(state.clone(), entry.id.to_string(), entry.name.to_string(), entry.url.to_string());
    Json(json!({"status": "started", "id": entry.id})).into_response()
}

#[derive(Deserialize)]
struct CustomDownloadReq {
    id: String,
    name: String,
    url: String,
}

async fn download_custom(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CustomDownloadReq>,
) -> Response {
    if !state.allow_remote_download {
        return json_err("загрузка из сети отключена", StatusCode::FORBIDDEN);
    }
    if !req.url.starts_with("http://") && !req.url.starts_with("https://") {
        return json_err("нужна ссылка http(s)", StatusCode::BAD_REQUEST);
    }
    let id = sanitize_id(&req.id);
    if id.is_empty() {
        return json_err("некорректный id", StatusCode::BAD_REQUEST);
    }
    spawn_download(state.clone(), id, req.name, req.url);
    Json(json!({"status": "started"})).into_response()
}

fn sanitize_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .take(64)
        .collect()
}

fn spawn_download(state: Arc<AppState>, id: String, name: String, url: String) {
    tokio::spawn(async move {
        match state.store.download(&id, &name, &url).await {
            Ok(path) => {
                tracing::info!("модель скачана: {}", path.display());
                let sha = ModelStore::sha256(&path).unwrap_or_default();
                let size = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
                let _ = state.db.conn().execute(
                    "INSERT OR REPLACE INTO models(id, name, file_path, size_bytes, sha256, params_json, source)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        id,
                        name,
                        path.to_string_lossy().to_string(),
                        size,
                        sha,
                        "{}",
                        "downloaded"
                    ],
                );
            }
            Err(e) => tracing::error!("ошибка скачивания модели {id}: {e}"),
        }
    });
}

async fn delete_model(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let id = sanitize_id(&id);
    state.engine.evict(&state.store.path_for(&id));
    if let Err(e) = state.store.delete(&id) {
        return json_err(&format!("ошибка удаления: {e}"), StatusCode::INTERNAL_SERVER_ERROR);
    }
    let _ = state
        .db
        .conn()
        .execute("DELETE FROM models WHERE id = ?1", rusqlite::params![id]);
    Json(json!({"status": "ok"})).into_response()
}

#[derive(Deserialize)]
struct ActivateReq {
    id: String,
}

async fn activate_model(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ActivateReq>,
) -> Response {
    let id = sanitize_id(&req.id);
    let path = state.store.path_for(&id);
    if !path.exists() {
        return json_err("файл модели не найден — сначала скачайте её", StatusCode::NOT_FOUND);
    }
    state.engine.set_active(&path);
    Json(json!({"status": "ok", "active": path.to_string_lossy()})) .into_response()
}

// ── параметры генерации ──

async fn get_params(State(state): State<Arc<AppState>>) -> Response {
    let mut v = match state.db.get_setting("gen_params") {
        Some(raw) => serde_json::from_str::<Value>(&raw)
            .unwrap_or_else(|_| serde_json::to_value(state.engine.default_params()).unwrap_or(json!({}))),
        None => serde_json::to_value(state.engine.default_params()).unwrap_or(json!({})),
    };
    // Если свой системный промпт не задан — показываем действующий по умолчанию,
    // чтобы его можно было прочитать и отредактировать.
    let empty = v["system_prompt"].as_str().map(|s| s.trim().is_empty()).unwrap_or(true);
    if empty {
        v["system_prompt"] = json!(analysis::DEFAULT_SYSTEM_PROMPT);
    }
    v["default_system_prompt"] = json!(analysis::DEFAULT_SYSTEM_PROMPT);
    Json(v).into_response()
}

async fn save_params(
    State(state): State<Arc<AppState>>,
    Json(params): Json<GenParams>,
) -> Response {
    let raw = serde_json::to_string(&params).unwrap_or_default();
    match state.db.set_setting("gen_params", &raw) {
        Ok(()) => Json(json!({"status": "ok", "params": params})).into_response(),
        Err(e) => json_err(&format!("ошибка сохранения: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_setting(State(state): State<Arc<AppState>>, Path(key): Path<String>) -> Response {
    match state.db.get_setting(&key) {
        Some(v) => Json(json!({"key": key, "value": v})).into_response(),
        None => Json(json!({"key": key, "value": Value::Null})).into_response(),
    }
}

async fn set_setting(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let value = match &body {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match state.db.set_setting(&key, &value) {
        Ok(()) => Json(json!({"status": "ok"})).into_response(),
        Err(e) => json_err(&format!("ошибка сохранения: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// ── генерация ──

/// Параметры запроса на генерацию.
#[derive(Deserialize, Clone)]
pub struct GenRequest {
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub messages: Vec<ChatMsg>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub params: Option<GenParams>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct ChatMsg {
    pub role: String,
    pub content: String,
}

/// Собрать список сообщений из запроса.
fn build_messages(req: &GenRequest, default_system: &str) -> Vec<ChatMessage> {
    let mut msgs: Vec<ChatMessage> = Vec::new();

    let sys = req
        .system
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| default_system.to_string());

    if !sys.trim().is_empty() {
        msgs.push(ChatMessage { role: "system".into(), content: sys });
    }

    if !req.messages.is_empty() {
        for m in &req.messages {
            msgs.push(ChatMessage { role: m.role.clone(), content: m.content.clone() });
        }
        return msgs;
    }

    if let Some(p) = &req.prompt {
        if !p.trim().is_empty() {
            msgs.push(ChatMessage { role: "user".into(), content: p.clone() });
        }
    }
    msgs
}

/// Определить путь активной модели.
fn resolve_model_path(state: &AppState, model_id: Option<&str>) -> Result<PathBuf, String> {
    // Явно переданная модель имеет приоритет.
    if let Some(id) = model_id {
        if !id.trim().is_empty() {
            let id = sanitize_id(id);
            let p = state.store.path_for(&id);
            if p.exists() {
                return Ok(p);
            }
            return Err(format!(
                "файл модели «{id}» не найден — скачайте его в разделе «Модели»"
            ));
        }
    }

    // Иначе — активная модель (хранится как полный путь к файлу).
    if let Some(p) = state.engine.active_model() {
        let path = PathBuf::from(&p);
        if path.exists() {
            return Ok(path);
        }
    }

    Err("не выбрана активная модель — скачайте и активируйте её в разделе «Модели»".to_string())
}

/// Загрузить сохранённые параметры и применить переопределения из запроса.
fn merged_params(state: &AppState, override_params: Option<&GenParams>) -> GenParams {
    let mut base = match state.db.get_setting("gen_params") {
        Some(raw) => serde_json::from_str::<GenParams>(&raw).unwrap_or_default(),
        None => state.engine.default_params().clone(),
    };
    if let Some(o) = override_params {
        base.temperature = o.temperature;
        base.top_p = o.top_p;
        base.top_k = o.top_k;
        base.repeat_penalty = o.repeat_penalty;
        base.dry_multiplier = o.dry_multiplier;
        base.max_tokens = o.max_tokens;
        base.n_ctx = o.n_ctx;
        base.n_threads = o.n_threads;
        base.seed = o.seed;
        if !o.system_prompt.trim().is_empty() {
            base.system_prompt = o.system_prompt.clone();
        }
    }
    base
}

async fn generate(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GenRequest>,
) -> Response {
    let path = match resolve_model_path(&state, req.model_id.as_deref()) {
        Ok(p) => p,
        Err(e) => return json_err(&e, StatusCode::BAD_REQUEST),
    };

    let params = merged_params(&state, req.params.as_ref());
    let default_system = if params.system_prompt.trim().is_empty() {
        analysis::DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        params.system_prompt.clone()
    };
    let msgs = build_messages(&req, &default_system);

    if msgs.is_empty() {
        return json_err("пустой запрос", StatusCode::BAD_REQUEST);
    }

    let engine = state.engine.clone();
    let model = path.clone();
    keepalive(async move {
        match engine.generate(&model, &msgs, &params).await {
            Ok(out) => Json(out).into_response(),
            Err(e) => {
                tracing::error!("generate: {e:#}");
                json_err(&format!("ошибка генерации: {e:#}"), StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    })
}

// ── анализ ──

#[derive(Deserialize)]
struct AnalyzeStudentReq {
    student_id: i64,
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    params: Option<GenParams>,
}

async fn analyze_student(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AnalyzeStudentReq>,
) -> Response {
    let context = match analysis::build_student_context(&state.db, req.student_id) {
        Ok(c) => c,
        Err(e) => return json_err(&format!("{e}"), StatusCode::BAD_REQUEST),
    };
    let prompt = analysis::build_report_prompt(&context, req.focus.as_deref());

    let gen = GenRequest {
        model_id: req.model_id,
        messages: vec![ChatMsg {
            role: "user".into(),
            content: prompt,
        }],
        prompt: None,
        system: None,
        params: req.params,
    };
    run_analysis(state, gen, Some(req.student_id)).await
}

#[derive(Deserialize)]
struct AnalyzeCompareReq {
    a: i64,
    b: i64,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    params: Option<GenParams>,
}

async fn analyze_compare(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AnalyzeCompareReq>,
) -> Response {
    let ca = match analysis::build_student_context(&state.db, req.a) {
        Ok(c) => c,
        Err(e) => return json_err(&format!("{e}"), StatusCode::BAD_REQUEST),
    };
    let cb = match analysis::build_student_context(&state.db, req.b) {
        Ok(c) => c,
        Err(e) => return json_err(&format!("{e}"), StatusCode::BAD_REQUEST),
    };
    let prompt = analysis::build_compare_prompt(&ca, &cb);

    let gen = GenRequest {
        model_id: req.model_id,
        messages: vec![ChatMsg {
            role: "user".into(),
            content: prompt,
        }],
        prompt: None,
        system: None,
        params: req.params,
    };
    run_analysis(state, gen, None).await
}

#[derive(Deserialize)]
struct AnalyzeAskReq {
    student_id: i64,
    question: String,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    params: Option<GenParams>,
}

/// Свободный вопрос по ученику: модель получает его полную статистику.
async fn analyze_ask(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AnalyzeAskReq>,
) -> Response {
    let context = match analysis::build_student_context(&state.db, req.student_id) {
        Ok(c) => c,
        Err(e) => return json_err(&format!("{e}"), StatusCode::BAD_REQUEST),
    };
    let prompt = analysis::build_freeform_prompt(&context, &req.question);

    let gen = GenRequest {
        model_id: req.model_id,
        messages: vec![ChatMsg {
            role: "user".into(),
            content: prompt,
        }],
        prompt: None,
        system: None,
        params: req.params,
    };
    run_analysis(state, gen, Some(req.student_id)).await
}

#[derive(Deserialize)]
struct AnalyzeOverviewReq {
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    params: Option<GenParams>,
}

async fn analyze_overview(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AnalyzeOverviewReq>,
) -> Response {
    let ov = match students::overview(&state.db) {
        Ok(o) => o,
        Err(e) => return json_err(&format!("{e}"), StatusCode::INTERNAL_SERVER_ERROR),
    };
    let summary = format!(
        "Учеников: {}\nУчебных записей (работы, оценки, занятия): {}\nСредний результат за работы: {}\nСредняя оценка (5-балльная шкала): {}\nПропущено уроков: {} (из них без уважительной причины: {})\nЗаписей в портфолио (достижения, грамоты, благодарности): {}\nПоведение: поощрений {}, замечаний и нарушений {}\n\nПо типам записей:\n{}",
        ov.students,
        ov.activities,
        ov.avg_ratio
            .map(|r| format!("{:.1}%", r * 100.0))
            .unwrap_or_else(|| "нет данных".into()),
        ov.avg_grade
            .map(|g| format!("{g:.2}"))
            .unwrap_or_else(|| "нет данных".into()),
        ov.absences,
        ov.absences_unexcused,
        ov.portfolio,
        ov.praises,
        ov.remarks,
        ov.kind_breakdown
            .iter()
            .map(|k| format!("  - {}: {} шт.", crate::labels::kind_ru(&k.kind), k.count))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let groups = ov
        .top_groups
        .iter()
        .map(|g| {
            format!(
                "  - {}: {} чел., средний результат {}",
                g.group_name,
                g.students,
                g.avg_ratio
                    .map(|r| format!("{:.0}%", r * 100.0))
                    .unwrap_or_else(|| "н/д".into())
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = analysis::build_overview_prompt(&summary, &groups);

    let gen = GenRequest {
        model_id: req.model_id,
        messages: vec![ChatMsg {
            role: "user".into(),
            content: prompt,
        }],
        prompt: None,
        system: None,
        params: req.params,
    };
    run_analysis(state, gen, None).await
}

/// Общая обёртка запуска анализа + сохранение в БД.
async fn run_analysis(
    state: Arc<AppState>,
    gen: GenRequest,
    student_id: Option<i64>,
) -> Response {
    let path = match resolve_model_path(&state, gen.model_id.as_deref()) {
        Ok(p) => p,
        Err(e) => return json_err(&e, StatusCode::BAD_REQUEST),
    };
    let params = merged_params(&state, gen.params.as_ref());
    let default_system = if params.system_prompt.trim().is_empty() {
        analysis::DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        params.system_prompt.clone()
    };
    let msgs = build_messages(&gen, &default_system);
    let engine = state.engine.clone();
    let model = path.clone();
    let model_id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    keepalive(async move {
        match engine.generate(&model, &msgs, &params).await {
            Ok(out) => {
                if let Some(sid) = student_id {
                    let prompt_text = gen
                        .messages
                        .first()
                        .map(|m| m.content.clone())
                        .unwrap_or_default();
                    let _ = state.db.conn().execute(
                        "INSERT INTO analyses(student_id, model_id, prompt, answer) VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![sid, model_id, prompt_text, out.text],
                    );
                }
                Json(out).into_response()
            }
            Err(e) => {
                tracing::error!("analysis: {e:#}");
                json_err(&format!("ошибка генерации: {e:#}"), StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    })
}

// ──────────────────────────────── Прочее ────────────────────────────────

/// Тихая остановка по Ctrl-C / SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("останавливаюсь");
}
