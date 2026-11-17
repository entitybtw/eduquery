//! eduquery — локальная LLM-платформа для анализа учебной деятельности учеников.
//!
//! Веб-интерфейс, инференс через llama-server (llama.cpp, скачивается автоматически), SQLite-хранилище.
//! Разворачивается через Docker Compose.

mod analysis;
mod auth;
mod bench;
mod config;
mod db;
mod llm;
mod models;
mod seed;
mod server;
mod students;
mod users;
mod labels;
mod admin_api;

use anyhow::Result;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use config::Config;
use db::Db;
use llm::{Engine, GenParams};
use models::ModelStore;
use server::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cfg = Config::from_env()?;
    tracing::info!(
        "eduquery v{} — данные в {}",
        env!("CARGO_PKG_VERSION"),
        cfg.data_dir.display()
    );

    // ── БД ──
    let db = Db::open(&cfg.db_path())?;
    let student_count: i64 = db.conn().query_row("SELECT COUNT(*) FROM students", [], |r| r.get(0))?;
    if student_count == 0 {
        let n = seed::seed(&db)?;
        tracing::info!("сгенерировано демо-данных: {n} активностей");
    }

    // ── LLM ──
    let default_params = GenParams {
        n_ctx: cfg.n_ctx,
        n_threads: cfg.threads(),
        ..Default::default()
    };
    let engine = Engine::new(default_params, cfg.data_dir.clone(), cfg.allow_remote_download)?;

    // ── модели ──
    let store = Arc::new(ModelStore::new(cfg.models_dir()));
    // Сверяем файлы на диске с таблицей models: после обновления/ручного
    // копирования модели должны быть видны без перезапуска вручную.
    if let Err(e) = store.reconcile(&db) {
        tracing::warn!("не удалось сверить модели с БД: {e:#}");
    }
    restore_active_model(&db, &engine, &store);

    // ── авторизация ──
    users::ensure_admin(&db, &cfg.admin_user, &cfg.admin_pass)?;
    let auth = Arc::new(auth::Auth::new(cfg.allow_anonymous));
    auth::restore_sessions(&db, &auth)?;

    // ── состояние ──
    let web_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("web")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("web"));

    let state = Arc::new(AppState {
        db,
        engine,
        store,
        auth,
        web_dir,
        allow_remote_download: cfg.allow_remote_download,
        bench: bench::BenchState::new(),
    });

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;
    server::serve(state, addr).await
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("eduquery=info,tower_http=warn,warn"));
    fmt().with_env_filter(filter).with_target(false).init();
}

/// Вернуть активную модель после перезапуска: сохранённый id, иначе первый
/// найденный GGUF. Без этого панель после рестарта оставалась «без модели».
fn restore_active_model(db: &Db, engine: &Engine, store: &ModelStore) {
    let saved = db.get_setting("active_model_id").unwrap_or_default();
    if !saved.trim().is_empty() {
        let path = store.path_for(saved.trim());
        if path.exists() {
            engine.set_active(&path);
            tracing::info!("активная модель восстановлена: {saved}");
            return;
        }
        let _ = db.set_setting("active_model_id", "");
    }
    if engine.active_model().is_none() {
        if let Some((id, _)) = store.list_files().first() {
            let path = store.path_for(id);
            engine.set_active(&path);
            let _ = db.set_setting("active_model_id", id);
            tracing::info!("активная модель назначена автоматически: {id}");
        }
    }
}
