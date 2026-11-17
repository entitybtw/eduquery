//! Бенчмарки: один запуск сравнивает несколько установленных моделей на одной
//! задаче. Генерация идёт в фоне, результаты сохраняются в БД, ответы можно
//! читать рядом и оценивать вручную по 5-балльной шкале.
//!
//! Задача может быть трёх видов:
//! * `student` — полный разбор ученика (как вкладка «Анализ ИИ»);
//! * `characteristic` — официальная характеристика;
//! * `custom` — произвольный запрос пользователя.
//!
//! Метрики (токены, скорость, время, зацикливание) пишутся и в результаты
//! бенчмарка, и в общий журнал `usage_metrics`: так модели можно сравнивать
//! не только по тестам, но и по реальному использованию.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::analysis;
use crate::llm::{ChatMessage, GenParams};
use crate::server::{
    compose_messages, merged_params, record_usage, record_usage_error, sanitize_id, AppState,
};
use crate::students;

/// Общее состояние бенчмарков.
pub struct BenchState {
    /// id запущенного бенчмарка или 0.
    running: AtomicI64,
}

impl BenchState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            running: AtomicI64::new(0),
        })
    }

    pub fn running_id(&self) -> i64 {
        self.running.load(Ordering::SeqCst)
    }
}

fn bench_err(msg: impl AsRef<str>, code: StatusCode) -> Response {
    (code, Json(json!({"error": msg.as_ref()}))).into_response()
}

// ─────────────────────────────── запуск ───────────────────────────────

#[derive(Deserialize)]
pub struct StartBenchReq {
    /// student | characteristic | custom
    #[serde(default)]
    mode: String,
    #[serde(default)]
    student_id: Option<i64>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    prompt: String,
    model_ids: Vec<String>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    example_input: Option<String>,
    #[serde(default)]
    example_answer: Option<String>,
    #[serde(default)]
    params: Option<Value>,
}

/// Запустить сравнение. Возвращает id запуска; сама генерация идёт в фоне.
pub async fn start_benchmark(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartBenchReq>,
) -> Response {
    if state.bench.running_id() != 0 {
        return bench_err("бенчмарк уже выполняется — дождитесь окончания", StatusCode::CONFLICT);
    }

    let mode = match req.mode.trim() {
        "student" => "student",
        "characteristic" => "characteristic",
        "custom" | "" => "custom",
        other => return bench_err(format!("неизвестный режим: {other}"), StatusCode::BAD_REQUEST),
    };

    // Модели: только установленные файлы, без дублей.
    let mut models: Vec<(String, String, std::path::PathBuf)> = Vec::new();
    for raw in req.model_ids.iter().take(12) {
        let id = sanitize_id(raw);
        if id.is_empty() || models.iter().any(|(m, _, _)| m == &id) {
            continue;
        }
        let path = state.store.path_for(&id);
        if !path.exists() {
            return bench_err(
                format!("модель «{id}» не найдена — сначала скачайте её"),
                StatusCode::BAD_REQUEST,
            );
        }
        let name = crate::models::catalog_entry(&id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|| id.clone());
        models.push((id, name, path));
    }
    if models.is_empty() {
        return bench_err("выберите хотя бы одну установленную модель", StatusCode::BAD_REQUEST);
    }

    // Задача и подпись запуска.
    let (prompt, default_title) = match mode {
        "student" => {
            let Some(sid) = req.student_id else {
                return bench_err("не выбран ученик", StatusCode::BAD_REQUEST);
            };
            let context = match analysis::build_student_context(&state.db, sid) {
                Ok(c) => c,
                Err(e) => return bench_err(format!("{e}"), StatusCode::BAD_REQUEST),
            };
            let name = students::get_student(&state.db, sid)
                .ok()
                .flatten()
                .map(|d| d.summary.full_name)
                .unwrap_or_else(|| format!("ученик #{sid}"));
            (
                analysis::build_report_prompt(&context, None),
                format!("Разбор ученика: {name}"),
            )
        }
        "characteristic" => {
            let Some(sid) = req.student_id else {
                return bench_err("не выбран ученик", StatusCode::BAD_REQUEST);
            };
            let context = match analysis::build_student_context(&state.db, sid) {
                Ok(c) => c,
                Err(e) => return bench_err(format!("{e}"), StatusCode::BAD_REQUEST),
            };
            let name = students::get_student(&state.db, sid)
                .ok()
                .flatten()
                .map(|d| d.summary.full_name)
                .unwrap_or_else(|| format!("ученик #{sid}"));
            (
                analysis::build_characteristic_prompt(&context),
                format!("Характеристика: {name}"),
            )
        }
        _ => {
            let p = req.prompt.trim().to_string();
            if p.is_empty() {
                return bench_err("пустой запрос", StatusCode::BAD_REQUEST);
            }
            let short: String = p.chars().take(70).collect();
            (p, format!("Свой запрос: {short}"))
        }
    };

    // Параметры: сохранённые настройки + переопределения из запроса.
    let mut params = merged_params(&state, req.params.as_ref());
    let system = req
        .system_prompt
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if params.system_prompt.trim().is_empty() {
                analysis::DEFAULT_SYSTEM_PROMPT.to_string()
            } else {
                params.system_prompt.clone()
            }
        });
    let example_input = req
        .example_input
        .clone()
        .unwrap_or_else(|| params.example_input.clone())
        .trim()
        .to_string();
    let example_answer = req
        .example_answer
        .clone()
        .unwrap_or_else(|| params.example_answer.clone())
        .trim()
        .to_string();
    params.system_prompt = system.clone();
    params.example_input = example_input.clone();
    params.example_answer = example_answer.clone();

    let title = if req.title.trim().is_empty() {
        default_title
    } else {
        req.title.trim().chars().take(160).collect()
    };
    let params_json = serde_json::to_string(&params).unwrap_or_else(|_| "{}".into());

    // Создаём запуск и строки результатов.
    let run_id: i64 = {
        let conn = state.db.conn();
        if let Err(e) = conn.execute(
            "INSERT INTO benchmark_runs(mode, title, prompt, student_id, system_prompt,
                                        example_input, example_answer, params_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                mode,
                title,
                prompt,
                req.student_id,
                system,
                example_input,
                example_answer,
                params_json
            ],
        ) {
            return bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR);
        }
        let rid = conn.last_insert_rowid();
        for (id, name, _) in &models {
            let _ = conn.execute(
                "INSERT INTO benchmark_results(run_id, model_id, model_name) VALUES (?1, ?2, ?3)",
                rusqlite::params![rid, id, name],
            );
        }
        rid
    };

    if state
        .bench
        .running
        .compare_exchange(0, run_id, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        let _ = state.db.conn().execute(
            "UPDATE benchmark_runs SET status = 'error', error = 'бенчмарк уже выполняется',
                                      finished_at = datetime('now') WHERE id = ?1",
            rusqlite::params![run_id],
        );
        return bench_err("бенчмарк уже выполняется", StatusCode::CONFLICT);
    }

    let messages = compose_messages(
        &params.system_prompt,
        &params.example_input,
        &params.example_answer,
        &[ChatMessage {
            role: "user".into(),
            content: prompt,
        }],
    );

    let state2 = state.clone();
    tokio::spawn(async move {
        run_benchmark(state2, run_id, models, messages, params).await;
    });

    Json(json!({"status": "started", "id": run_id})).into_response()
}

async fn run_benchmark(
    state: Arc<AppState>,
    run_id: i64,
    models: Vec<(String, String, std::path::PathBuf)>,
    messages: Vec<ChatMessage>,
    params: GenParams,
) {
    // Если задача паникует или её прервут, флаг «выполняется» нельзя оставлять
    // навсегда: иначе новые запуски будут получать 409 до перезапуска сервера.
    struct Guard {
        state: Arc<AppState>,
        run_id: i64,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            let conn = self.state.db.conn();
            let _ = conn.execute(
                "UPDATE benchmark_results SET status = 'error', error = 'прервано'
                 WHERE run_id = ?1 AND status IN ('pending', 'running')",
                rusqlite::params![self.run_id],
            );
            let _ = conn.execute(
                "UPDATE benchmark_runs SET status = 'error', error = 'бенчмарк прерван',
                                          finished_at = datetime('now')
                 WHERE id = ?1 AND status = 'running'",
                rusqlite::params![self.run_id],
            );
            self.state.bench.running.store(0, Ordering::SeqCst);
        }
    }
    let _guard = Guard { state: state.clone(), run_id };

    let mut errors = 0usize;
    for (id, name, path) in models {
        let _ = state.db.conn().execute(
            "UPDATE benchmark_results SET status = 'running' WHERE run_id = ?1 AND model_id = ?2",
            rusqlite::params![run_id, id],
        );
        let result_id: Option<i64> = state
            .db
            .conn()
            .query_row(
                "SELECT id FROM benchmark_results WHERE run_id = ?1 AND model_id = ?2",
                rusqlite::params![run_id, id],
                |r| r.get(0),
            )
            .ok();

        match state.engine.generate(&path, &messages, &params).await {
            Ok(out) => {
                record_usage(&state.db, &id, "benchmark", &out);
                if let Some(rid) = result_id {
                    let _ = state.db.conn().execute(
                        "UPDATE benchmark_results SET status = 'ok', answer = ?1,
                            prompt_tokens = ?2, completion_tokens = ?3, tokens_per_sec = ?4,
                            duration_ms = ?5, loop_detected = ?6
                         WHERE id = ?7",
                        rusqlite::params![
                            out.text,
                            out.prompt_tokens as i64,
                            out.completion_tokens as i64,
                            out.tokens_per_sec,
                            out.duration_ms as i64,
                            out.loop_detected as i64,
                            rid
                        ],
                    );
                }
                tracing::info!("бенчмарк #{run_id}: {name} — {} ток/с", out.tokens_per_sec.round());
            }
            Err(e) => {
                errors += 1;
                let msg = format!("{e:#}");
                record_usage_error(&state.db, &id, "benchmark", &msg);
                if let Some(rid) = result_id {
                    let _ = state.db.conn().execute(
                        "UPDATE benchmark_results SET status = 'error', error = ?1 WHERE id = ?2",
                        rusqlite::params![msg, rid],
                    );
                }
                tracing::warn!("бенчмарк #{run_id}: {name} — ошибка: {msg}");
            }
        }
    }

    let total = state
        .db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM benchmark_results WHERE run_id = ?1",
            rusqlite::params![run_id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0);
    let error_note = if errors > 0 && errors as i64 == total {
        "все модели завершились с ошибкой".to_string()
    } else {
        String::new()
    };
    let _ = state.db.conn().execute(
        "UPDATE benchmark_runs SET status = 'done', finished_at = datetime('now'), error = ?1
         WHERE id = ?2",
        rusqlite::params![error_note, run_id],
    );
    state.bench.running.store(0, Ordering::SeqCst);
}

// ─────────────────────────────── чтение ───────────────────────────────

/// Текущий прогресс: сколько моделей готово, что выполняется сейчас.
pub async fn bench_status(State(state): State<Arc<AppState>>) -> Response {
    let running = state.bench.running_id();
    if running == 0 {
        return Json(json!({"running": 0, "progress": null})).into_response();
    }
    let conn = state.db.conn();
    let run: Option<(String, String, String)> = conn
        .query_row(
            "SELECT title, mode, created_at FROM benchmark_runs WHERE id = ?1",
            rusqlite::params![running],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    let Some((title, mode, created_at)) = run else {
        return Json(json!({"running": 0, "progress": null})).into_response();
    };
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM benchmark_results WHERE run_id = ?1",
            rusqlite::params![running],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let done: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM benchmark_results WHERE run_id = ?1 AND status IN ('ok','error')",
            rusqlite::params![running],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let current: Option<String> = conn
        .query_row(
            "SELECT model_name FROM benchmark_results WHERE run_id = ?1 AND status = 'running'
             ORDER BY id LIMIT 1",
            rusqlite::params![running],
            |r| r.get(0),
        )
        .ok();
    Json(json!({
        "running": running,
        "progress": {
            "run_id": running,
            "title": title,
            "mode": mode,
            "created_at": created_at,
            "total": total,
            "done": done,
            "current": current,
        }
    }))
    .into_response()
}

/// История запусков + сводка реального использования моделей.
pub async fn list_benchmarks(State(state): State<Arc<AppState>>) -> Response {
    let conn = state.db.conn();
    let runs: Vec<serde_json::Value> = {
        let mut stmt = match conn.prepare(
            "SELECT r.id, r.created_at, r.finished_at, r.status, r.mode, r.title, r.student_id,
                    (SELECT COUNT(*) FROM benchmark_results x WHERE x.run_id = r.id),
                    (SELECT COUNT(*) FROM benchmark_results x WHERE x.run_id = r.id AND x.status = 'ok'),
                    (SELECT COUNT(*) FROM benchmark_results x WHERE x.run_id = r.id AND x.status = 'error'),
                    (SELECT AVG(NULLIF(x.rating, 0)) FROM benchmark_results x WHERE x.run_id = r.id),
                    (SELECT AVG(NULLIF(x.tokens_per_sec, 0)) FROM benchmark_results x WHERE x.run_id = r.id AND x.status = 'ok'),
                    (SELECT SUM(x.completion_tokens) FROM benchmark_results x WHERE x.run_id = r.id)
             FROM benchmark_runs r ORDER BY r.id DESC LIMIT 100",
        ) {
            Ok(s) => s,
            Err(e) => {
                return bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
            }
        };
        let it = stmt.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "created_at": r.get::<_, String>(1)?,
                "finished_at": r.get::<_, Option<String>>(2)?,
                "status": r.get::<_, String>(3)?,
                "mode": r.get::<_, String>(4)?,
                "title": r.get::<_, String>(5)?,
                "student_id": r.get::<_, Option<i64>>(6)?,
                "total": r.get::<_, i64>(7)?,
                "ok": r.get::<_, i64>(8)?,
                "errors": r.get::<_, i64>(9)?,
                "avg_rating": r.get::<_, Option<f64>>(10)?,
                "avg_tps": r.get::<_, Option<f64>>(11)?,
                "tokens": r.get::<_, Option<i64>>(12)?,
            }))
        });
        match it {
            Ok(rows) => rows.filter_map(|x| x.ok()).collect(),
            Err(e) => {
                return bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    };

    let usage: Vec<serde_json::Value> = {
        let mut stmt = match conn.prepare(
            "SELECT model_id, COUNT(*), SUM(prompt_tokens), SUM(completion_tokens),
                    AVG(NULLIF(tokens_per_sec, 0)), SUM(loop_detected), SUM(CASE WHEN ok = 0 THEN 1 ELSE 0 END),
                    MAX(created_at)
             FROM usage_metrics GROUP BY model_id ORDER BY COUNT(*) DESC, model_id",
        ) {
            Ok(s) => s,
            Err(e) => {
                return bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
            }
        };
        let it = stmt.query_map([], |r| {
            Ok(json!({
                "model_id": r.get::<_, String>(0)?,
                "calls": r.get::<_, i64>(1)?,
                "prompt_tokens": r.get::<_, Option<i64>>(2)?,
                "completion_tokens": r.get::<_, Option<i64>>(3)?,
                "avg_tps": r.get::<_, Option<f64>>(4)?,
                "loops": r.get::<_, Option<i64>>(5)?,
                "errors": r.get::<_, Option<i64>>(6)?,
                "last_used": r.get::<_, Option<String>>(7)?,
            }))
        });
        match it {
            Ok(rows) => rows.filter_map(|x| x.ok()).collect(),
            Err(e) => {
                return bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    };

    Json(json!({
        "runs": runs,
        "usage": usage,
        "running": state.bench.running_id(),
    }))
    .into_response()
}

/// Один запуск со всеми ответами.
pub async fn get_benchmark(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Response {
    let conn = state.db.conn();
    let run = match conn
        .query_row(
            "SELECT id, created_at, finished_at, status, mode, title, prompt, student_id,
                    system_prompt, example_input, example_answer, params_json, error
             FROM benchmark_runs WHERE id = ?1",
            rusqlite::params![id],
            |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "created_at": r.get::<_, String>(1)?,
                    "finished_at": r.get::<_, Option<String>>(2)?,
                    "status": r.get::<_, String>(3)?,
                    "mode": r.get::<_, String>(4)?,
                    "title": r.get::<_, String>(5)?,
                    "prompt": r.get::<_, String>(6)?,
                    "student_id": r.get::<_, Option<i64>>(7)?,
                    "system_prompt": r.get::<_, String>(8)?,
                    "example_input": r.get::<_, String>(9)?,
                    "example_answer": r.get::<_, String>(10)?,
                    "params_json": r.get::<_, String>(11)?,
                    "error": r.get::<_, String>(12)?,
                }))
            },
        )
        .optional()
    {
        Ok(Some(v)) => v,
        Ok(None) => return bench_err("запуск не найден", StatusCode::NOT_FOUND),
        Err(e) => {
            return bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
        }
    };

    let results: Vec<serde_json::Value> = {
        let mut stmt = match conn.prepare(
            "SELECT id, model_id, model_name, status, error, answer, prompt_tokens,
                    completion_tokens, tokens_per_sec, duration_ms, loop_detected, rating, created_at
             FROM benchmark_results WHERE run_id = ?1 ORDER BY id",
        ) {
            Ok(s) => s,
            Err(e) => {
                return bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
            }
        };
        let it = stmt.query_map(rusqlite::params![id], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "model_id": r.get::<_, String>(1)?,
                "model_name": r.get::<_, String>(2)?,
                "status": r.get::<_, String>(3)?,
                "error": r.get::<_, String>(4)?,
                "answer": r.get::<_, String>(5)?,
                "prompt_tokens": r.get::<_, i64>(6)?,
                "completion_tokens": r.get::<_, i64>(7)?,
                "tokens_per_sec": r.get::<_, f64>(8)?,
                "duration_ms": r.get::<_, i64>(9)?,
                "loop_detected": r.get::<_, i64>(10)?,
                "rating": r.get::<_, i64>(11)?,
                "created_at": r.get::<_, String>(12)?,
            }))
        });
        match it {
            Ok(rows) => rows.filter_map(|x| x.ok()).collect(),
            Err(e) => {
                return bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    };

    Json(json!({"run": run, "results": results})).into_response()
}

// ─────────────────────────────── действия ───────────────────────────────

#[derive(Deserialize)]
pub struct RatingReq {
    #[serde(default)]
    rating: i64,
}

/// Ручная оценка ответа: 1–5, 0 — сбросить.
pub async fn rate_result(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(req): Json<RatingReq>,
) -> Response {
    let rating = req.rating.clamp(0, 5);
    match state.db.conn().execute(
        "UPDATE benchmark_results SET rating = ?1 WHERE id = ?2",
        rusqlite::params![rating, id],
    ) {
        Ok(0) => bench_err("результат не найден", StatusCode::NOT_FOUND),
        Ok(_) => Json(json!({"status": "ok", "rating": rating})).into_response(),
        Err(e) => bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Удалить запуск вместе с результатами.
pub async fn delete_benchmark(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Response {
    if state.bench.running_id() == id {
        return bench_err("нельзя удалить выполняющийся бенчмарк", StatusCode::CONFLICT);
    }
    match state
        .db
        .conn()
        .execute("DELETE FROM benchmark_runs WHERE id = ?1", rusqlite::params![id])
    {
        Ok(0) => bench_err("запуск не найден", StatusCode::NOT_FOUND),
        Ok(_) => Json(json!({"status": "ok"})).into_response(),
        Err(e) => bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Выгрузка всех запусков и ответов в CSV.
pub async fn export_benchmarks(State(state): State<Arc<AppState>>) -> Response {
    let conn = state.db.conn();
    let mut stmt = match conn.prepare(
        "SELECT r.id, r.created_at, r.mode, r.title, r.status, x.model_name, x.status,
                x.rating, x.prompt_tokens, x.completion_tokens, x.tokens_per_sec,
                x.duration_ms, x.loop_detected, x.error, x.answer
         FROM benchmark_runs r JOIN benchmark_results x ON x.run_id = r.id
         ORDER BY r.id DESC, x.id",
    ) {
        Ok(s) => s,
        Err(e) => return bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
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
            r.get::<_, i64>(7)?.to_string(),
            r.get::<_, i64>(8)?.to_string(),
            r.get::<_, i64>(9)?.to_string(),
            format!("{:.2}", r.get::<_, f64>(10)?),
            r.get::<_, i64>(11)?.to_string(),
            if r.get::<_, i64>(12)? != 0 { "да".to_string() } else { "нет".to_string() },
            r.get::<_, String>(13)?,
            r.get::<_, String>(14)?,
        ])
    });
    let mut rows = vec![vec![
        "№ запуска".into(),
        "Дата".into(),
        "Режим".into(),
        "Название".into(),
        "Статус запуска".into(),
        "Модель".into(),
        "Статус".into(),
        "Оценка".into(),
        "Токенов запроса".into(),
        "Токенов ответа".into(),
        "Токенов в секунду".into(),
        "Время, мс".into(),
        "Зацикливание".into(),
        "Ошибка".into(),
        "Ответ".into(),
    ]];
    match mapped {
        Ok(it) => rows.extend(it.filter_map(|r| r.ok())),
        Err(e) => return bench_err(format!("ошибка БД: {e}"), StatusCode::INTERNAL_SERVER_ERROR),
    }
    crate::admin_api::csv_response("бенчмарки", rows)
}
