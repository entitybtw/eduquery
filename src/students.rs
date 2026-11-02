//! Работа с учениками: список, карточка, агрегаты, импорт данных.

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Db;

/// Краткая карточка ученика в списке.
#[derive(Debug, Clone, Serialize)]
pub struct StudentSummary {
    pub id: i64,
    pub external_id: String,
    pub full_name: String,
    pub group_name: String,
    pub email: String,
    pub grade: Option<i64>,
    pub activities: i64,
    pub avg_score: Option<f64>,
    pub avg_ratio: Option<f64>,
    pub total_minutes: Option<f64>,
    pub last_activity: Option<String>,
}

/// Полная карточка ученика.
#[derive(Debug, Clone, Serialize)]
pub struct StudentDetail {
    #[serde(flatten)]
    pub summary: StudentSummary,
    pub by_subject: Vec<SubjectStat>,
    pub by_kind: Vec<KindStat>,
    pub timeline: Vec<TimelinePoint>,
    pub recent: Vec<Activity>,
    pub strengths: Vec<String>,
    pub weaknesses: Vec<String>,
}

/// Агрегат по предмету.
#[derive(Debug, Clone, Serialize)]
pub struct SubjectStat {
    pub subject: String,
    pub count: i64,
    pub avg_score: Option<f64>,
    pub avg_ratio: Option<f64>,
    pub total_minutes: Option<f64>,
}

/// Агрегат по типу активности.
#[derive(Debug, Clone, Serialize)]
pub struct KindStat {
    pub kind: String,
    pub count: i64,
    pub avg_ratio: Option<f64>,
}

/// Точка временного ряда.
#[derive(Debug, Clone, Serialize)]
pub struct TimelinePoint {
    pub period: String,
    pub count: i64,
    pub avg_ratio: Option<f64>,
}

/// Одна активность.
#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub subject: String,
    pub score: Option<f64>,
    pub max_score: Option<f64>,
    pub grade: Option<i64>,
    pub status: String,
    pub duration_min: Option<f64>,
    pub occurred_at: String,
    pub details: Value,
}

/// Ученик по идентификатору БД.
pub fn get_student(db: &Db, id: i64) -> Result<Option<StudentDetail>> {
    let conn = db.conn();
    let summary: Option<StudentSummary> = conn
        .query_row(
            "SELECT s.id, s.external_id, s.full_name, s.group_name, s.email, s.grade,
                    (SELECT COUNT(*) FROM activities a WHERE a.student_id = s.id) AS acts,
                    (SELECT AVG(a.score) FROM activities a WHERE a.student_id = s.id AND a.score IS NOT NULL) AS avg_score,
                    (SELECT AVG(a.score / a.max_score) FROM activities a
                       WHERE a.student_id = s.id AND a.max_score IS NOT NULL AND a.max_score > 0) AS avg_ratio,
                    (SELECT SUM(a.duration_min) FROM activities a WHERE a.student_id = s.id) AS total_min,
                    (SELECT MAX(a.occurred_at) FROM activities a WHERE a.student_id = s.id) AS last_at
             FROM students s WHERE s.id = ?1",
            params![id],
            |r| {
                Ok(StudentSummary {
                    id: r.get(0)?,
                    external_id: r.get(1)?,
                    full_name: r.get(2)?,
                    group_name: r.get(3)?,
                    email: r.get(4)?,
                    grade: r.get(5)?,
                    activities: r.get(6)?,
                    avg_score: r.get(7)?,
                    avg_ratio: r.get(8)?,
                    total_minutes: r.get(9)?,
                    last_activity: r.get(10)?,
                })
            },
        )
        .optional()?;

    let Some(summary) = summary else {
        return Ok(None);
    };

    let by_subject: Vec<SubjectStat> = {
        let mut stmt = conn.prepare(
            "SELECT subject, COUNT(*) AS c,
                    AVG(score) AS avg_score,
                    AVG(score / max_score) AS avg_ratio,
                    SUM(duration_min) AS total_min
             FROM activities
             WHERE student_id = ?1 AND subject <> ''
             GROUP BY subject ORDER BY c DESC, subject ASC",
        )?;
        let rows = stmt.query_map(params![id], |r| {
            Ok(SubjectStat {
                subject: r.get(0)?,
                count: r.get(1)?,
                avg_score: r.get(2)?,
                avg_ratio: r.get(3)?,
                total_minutes: r.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let by_kind: Vec<KindStat> = {
        let mut stmt = conn.prepare(
            "SELECT kind, COUNT(*) AS c, AVG(score / max_score) AS avg_ratio
             FROM activities WHERE student_id = ?1
             GROUP BY kind ORDER BY c DESC",
        )?;
        let rows = stmt.query_map(params![id], |r| {
            Ok(KindStat {
                kind: r.get(0)?,
                count: r.get(1)?,
                avg_ratio: r.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let timeline: Vec<TimelinePoint> = {
        let mut stmt = conn.prepare(
            "SELECT substr(occurred_at, 1, 7) AS period, COUNT(*) AS c,
                    AVG(score / max_score) AS avg_ratio
             FROM activities WHERE student_id = ?1
             GROUP BY period ORDER BY period ASC",
        )?;
        let rows = stmt.query_map(params![id], |r| {
            Ok(TimelinePoint {
                period: r.get(0)?,
                count: r.get(1)?,
                avg_ratio: r.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let recent: Vec<Activity> = {
        let mut stmt = conn.prepare(
            "SELECT id, kind, title, subject, score, max_score, grade, status,
                    duration_min, occurred_at, details
             FROM activities WHERE student_id = ?1
             ORDER BY occurred_at DESC LIMIT 60",
        )?;
        let rows = stmt.query_map(params![id], map_activity)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut detail = StudentDetail {
        summary,
        by_subject,
        by_kind,
        timeline,
        recent,
        strengths: Vec::new(),
        weaknesses: Vec::new(),
    };

    // Простые эвристики для предпросмотра, ИИ докручивает детали.
    if let Some(ratio) = detail.summary.avg_ratio {
        if ratio >= 0.8 {
            detail.strengths.push(format!(
                "Высокий средний результат ({:.0}% от максимума)",
                ratio * 100.0
            ));
        } else if ratio < 0.55 {
            detail.weaknesses.push(format!(
                "Низкий средний результат ({:.0}% от максимума)",
                ratio * 100.0
            ));
        }
    }
    if let Some(min) = detail.summary.total_minutes {
        if min < 60.0 {
            detail.weaknesses.push("Мало времени, зафиксированного за работами".into());
        } else if min > 1200.0 {
            detail.strengths.push("Большой объём практики".into());
        }
    }
    if let Some(last) = &detail.summary.last_activity {
        detail
            .strengths
            .push(format!("Последняя активность: {last}"));
    }

    Ok(Some(detail))
}

fn map_activity(r: &Row<'_>) -> rusqlite::Result<Activity> {
    let details_raw: String = r.get(10)?;
    Ok(Activity {
        id: r.get(0)?,
        kind: r.get(1)?,
        title: r.get(2)?,
        subject: r.get(3)?,
        score: r.get(4)?,
        max_score: r.get(5)?,
        grade: r.get(6)?,
        status: r.get(7)?,
        duration_min: r.get(8)?,
        occurred_at: r.get(9)?,
        details: serde_json::from_str(&details_raw).unwrap_or(Value::Null),
    })
}

/// Список учеников с поиском и сортировкой.
pub fn list_students(
    db: &Db,
    query: &str,
    group: &str,
    sort: &str,
) -> Result<Vec<StudentSummary>> {
    let conn = db.conn();
    let mut sql = String::from(
        "SELECT s.id, s.external_id, s.full_name, s.group_name, s.email, s.grade,
                (SELECT COUNT(*) FROM activities a WHERE a.student_id = s.id) AS acts,
                (SELECT AVG(a.score) FROM activities a WHERE a.student_id = s.id AND a.score IS NOT NULL) AS avg_score,
                (SELECT AVG(a.score / a.max_score) FROM activities a
                   WHERE a.student_id = s.id AND a.max_score IS NOT NULL AND a.max_score > 0) AS avg_ratio,
                (SELECT SUM(a.duration_min) FROM activities a WHERE a.student_id = s.id) AS total_min,
                (SELECT MAX(a.occurred_at) FROM activities a WHERE a.student_id = s.id) AS last_at
         FROM students s WHERE 1=1",
    );

    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if !query.is_empty() {
        sql.push_str(" AND (s.full_name LIKE ? OR s.external_id LIKE ? OR s.group_name LIKE ?)");
        let like = format!("%{query}%");
        args.push(Box::new(like.clone()));
        args.push(Box::new(like.clone()));
        args.push(Box::new(like));
    }
    if !group.is_empty() {
        sql.push_str(" AND s.group_name = ?");
        args.push(Box::new(group.to_string()));
    }

    sql.push_str(match sort {
        "name" => " ORDER BY s.full_name ASC",
        "ratio" => " ORDER BY avg_ratio DESC NULLS LAST, s.full_name ASC",
        "ratio_asc" => " ORDER BY avg_ratio ASC NULLS LAST, s.full_name ASC",
        "activities" => " ORDER BY acts DESC, s.full_name ASC",
        "recent" => " ORDER BY last_at DESC NULLS LAST",
        _ => " ORDER BY s.id ASC",
    });

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), |r| {
        Ok(StudentSummary {
            id: r.get(0)?,
            external_id: r.get(1)?,
            full_name: r.get(2)?,
            group_name: r.get(3)?,
            email: r.get(4)?,
            grade: r.get(5)?,
            activities: r.get(6)?,
            avg_score: r.get(7)?,
            avg_ratio: r.get(8)?,
            total_minutes: r.get(9)?,
            last_activity: r.get(10)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Список групп.
pub fn list_groups(db: &Db) -> Result<Vec<String>> {
    let conn = db.conn();
    let mut stmt = conn.prepare("SELECT DISTINCT group_name FROM students WHERE group_name <> '' ORDER BY group_name")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Сводка по всем ученикам.
#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub students: i64,
    pub activities: i64,
    pub avg_ratio: Option<f64>,
    pub avg_minutes: Option<f64>,
    pub top_groups: Vec<GroupStat>,
    pub kind_breakdown: Vec<KindStat>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupStat {
    pub group_name: String,
    pub students: i64,
    pub avg_ratio: Option<f64>,
}

pub fn overview(db: &Db) -> Result<Overview> {
    let conn = db.conn();
    let students: i64 = conn.query_row("SELECT COUNT(*) FROM students", [], |r| r.get(0))?;
    let activities: i64 = conn.query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0))?;
    let avg_ratio: Option<f64> = conn.query_row(
        "SELECT AVG(score / max_score) FROM activities WHERE max_score IS NOT NULL AND max_score > 0",
        [],
        |r| r.get(0),
    )?;
    let avg_minutes: Option<f64> =
        conn.query_row("SELECT AVG(total) FROM (SELECT SUM(duration_min) total FROM activities GROUP BY student_id)", [], |r| r.get(0))?;

    let top_groups: Vec<GroupStat> = {
        let mut stmt = conn.prepare(
            "SELECT s.group_name, COUNT(DISTINCT s.id) AS n,
                    AVG(a.score / a.max_score) AS avg_ratio
             FROM students s JOIN activities a ON a.student_id = s.id
             WHERE s.group_name <> '' AND a.max_score IS NOT NULL AND a.max_score > 0
             GROUP BY s.group_name ORDER BY avg_ratio DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(GroupStat {
                group_name: r.get(0)?,
                students: r.get(1)?,
                avg_ratio: r.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let kind_breakdown: Vec<KindStat> = {
        let mut stmt = conn.prepare(
            "SELECT kind, COUNT(*) c, AVG(score / max_score) avg_ratio FROM activities GROUP BY kind ORDER BY c DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(KindStat {
                kind: r.get(0)?,
                count: r.get(1)?,
                avg_ratio: r.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    Ok(Overview {
        students,
        activities,
        avg_ratio,
        avg_minutes,
        top_groups,
        kind_breakdown,
    })
}

// ───────────────────────────── Импорт данных ─────────────────────────────

/// Одна запись, приходящая из импорта.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportRow {
    #[serde(alias = "student_id", alias = "внешний_id", alias = "id")]
    pub external_id: Option<String>,
    #[serde(alias = "name", alias = "фио", alias = "ФИО")]
    pub full_name: Option<String>,
    #[serde(alias = "group", alias = "группа", alias = "class")]
    pub group_name: Option<String>,
    #[serde(alias = "email", alias = "почта")]
    pub email: Option<String>,
    #[serde(alias = "course", alias = "предмет", alias = "subject", alias = "дисциплина")]
    pub subject: Option<String>,
    #[serde(alias = "task", alias = "задание", alias = "title", alias = "название")]
    pub title: Option<String>,
    #[serde(alias = "type", alias = "тип", alias = "kind")]
    pub kind: Option<String>,
    #[serde(alias = "score", alias = "балл", alias = "оценка")]
    pub score: Option<f64>,
    #[serde(alias = "max_score", alias = "макс_балл", alias = "max")]
    pub max_score: Option<f64>,
    #[serde(alias = "grade", alias = "оценка_итог")]
    pub grade: Option<i64>,
    #[serde(alias = "status", alias = "статус")]
    pub status: Option<String>,
    #[serde(alias = "duration", alias = "длительность", alias = "minutes")]
    pub duration_min: Option<f64>,
    #[serde(alias = "date", alias = "дата", alias = "occurred_at", alias = "timestamp")]
    pub occurred_at: Option<String>,
}

/// Результат импорта.
#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub rows_read: usize,
    pub students_created: usize,
    pub students_updated: usize,
    pub activities_created: usize,
    pub errors: Vec<String>,
}

/// Импортировать CSV. Определяет, есть ли в файле идентификатор ученика.
pub fn import_csv(db: &Db, content: &str) -> Result<ImportResult> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(content.as_bytes());

    let headers = reader
        .headers()
        .context("не удалось прочитать заголовки CSV")?
        .clone();

    let mut result = ImportResult {
        rows_read: 0,
        students_created: 0,
        students_updated: 0,
        activities_created: 0,
        errors: Vec::new(),
    };

    for (idx, rec) in reader.records().enumerate() {
        let rec = match rec {
            Ok(r) => r,
            Err(e) => {
                result.errors.push(format!("строка {}: {e}", idx + 2));
                continue;
            }
        };
        result.rows_read += 1;

        let mut map = serde_json::Map::new();
        for (h, v) in headers.iter().zip(rec.iter()) {
            map.insert(h.to_lowercase(), Value::String(v.to_string()));
        }
        let row: ImportRow = match serde_json::from_value(Value::Object(map)) {
            Ok(r) => r,
            Err(e) => {
                result.errors.push(format!("строка {}: не разобрана ({e})", idx + 2));
                continue;
            }
        };

        match upsert_student(db, &row, &mut result) {
            Ok(Some(student_id)) => {
                if let Err(e) = insert_activity(db, student_id, &row) {
                    result.errors.push(format!("строка {}: {e}", idx + 2));
                } else {
                    result.activities_created += 1;
                }
            }
            Ok(None) => {
                result.errors.push(format!(
                    "строка {}: не удалось определить ученика (нужны external_id или full_name)",
                    idx + 2
                ));
            }
            Err(e) => result.errors.push(format!("строка {}: {e}", idx + 2)),
        }
    }

    Ok(result)
}

/// Публичная обёртка: создать/обновить ученика по строке импорта.
pub fn upsert_row(db: &Db, row: &ImportRow, result: &mut ImportResult) -> Result<Option<i64>> {
    upsert_student(db, row, result)
}

/// Публичная обёртка: добавить активность по строке импорта.
pub fn insert_row(db: &Db, student_id: i64, row: &ImportRow) -> Result<()> {
    insert_activity(db, student_id, row)
}

fn upsert_student(
    db: &Db,
    row: &ImportRow,
    result: &mut ImportResult,
) -> Result<Option<i64>> {
    let conn = db.conn();
    let external_id = row
        .external_id
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let full_name = row
        .full_name
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    if external_id.is_none() && full_name.is_none() {
        return Ok(None);
    }

    // Ищем по external_id, иначе по имени.
    let existing: Option<i64> = if let Some(eid) = external_id {
        conn.query_row("SELECT id FROM students WHERE external_id = ?1", params![eid], |r| r.get(0))
            .optional()?
    } else {
        conn.query_row(
            "SELECT id FROM students WHERE full_name = ?1 ORDER BY id LIMIT 1",
            params![full_name.unwrap()],
            |r| r.get(0),
        )
        .optional()?
    };

    let group = row.group_name.clone().unwrap_or_default();
    let email = row.email.clone().unwrap_or_default();

    if let Some(id) = existing {
        conn.execute(
            "UPDATE students SET
                full_name = COALESCE(NULLIF(?2,''), full_name),
                group_name = CASE WHEN ?3 <> '' THEN ?3 ELSE group_name END,
                email      = CASE WHEN ?4 <> '' THEN ?4 ELSE email END,
                updated_at = datetime('now')
             WHERE id = ?1",
            params![id, full_name.unwrap_or(""), group, email],
        )?;
        result.students_updated += 1;
        return Ok(Some(id));
    }

    // Создаём нового. external_id генерируем, если его нет.
    let eid = match external_id {
        Some(e) => e.to_string(),
        None => format!("AUTO-{:04}", conn.query_row("SELECT COUNT(*) FROM students", [], |r| r.get::<_, i64>(0))? + 1),
    };
    let name = full_name.unwrap_or(&eid).to_string();

    conn.execute(
        "INSERT INTO students(external_id, full_name, group_name, email) VALUES (?1, ?2, ?3, ?4)",
        params![eid, name, group, email],
    )?;
    result.students_created += 1;
    Ok(Some(conn.last_insert_rowid()))
}

fn insert_activity(db: &Db, student_id: i64, row: &ImportRow) -> Result<()> {
    let conn = db.conn();
    let kind = row
        .kind
        .as_ref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "task".to_string());
    let kind = match kind.as_str() {
        "task" | "homework" | "дз" | "задание" => "task",
        "quiz" | "test" | "тест" | "контрольная" => "quiz",
        "submission" | "project" | "проект" => "submission",
        "attendance" | "посещаемость" => "attendance",
        "grade" | "оценка" => "grade",
        other => other,
    };

    let title = row
        .title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("Активность ({kind})"));
    let subject = row.subject.clone().unwrap_or_default();

    let max_score = row.max_score.filter(|m| *m > 0.0);
    let score = row.score.filter(|_| max_score.is_some());

    let occurred_at = match row.occurred_at.as_ref().map(|s| s.trim()) {
        Some(s) if !s.is_empty() => normalize_date(s),
        _ => chrono_now(),
    };

    let status = row
        .status
        .clone()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "done".to_string());

    conn.execute(
        "INSERT INTO activities(student_id, kind, title, subject, score, max_score,
                                grade, status, duration_min, occurred_at, details)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            student_id,
            kind,
            title,
            subject,
            score,
            max_score,
            row.grade,
            status,
            row.duration_min,
            occurred_at,
            "{}"
        ],
    )?;
    Ok(())
}

/// Привести дату к формату SQLite (`YYYY-MM-DD HH:MM:SS`).
fn normalize_date(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return chrono_now();
    }
    // Уже похоже на ISO.
    if s.len() >= 10 && s.chars().take(4).all(|c| c.is_ascii_digit()) {
        let base = &s[..10];
        let rest = if s.len() > 10 {
            s[10..].trim_start_matches(['T', ' '])
        } else {
            ""
        };
        let time = if rest.is_empty() {
            "00:00:00".to_string()
        } else if rest.len() >= 8 && rest.contains(':') {
            rest[..8].to_string()
        } else {
            "00:00:00".to_string()
        };
        return format!("{base} {time}");
    }
    // ISO 8601 с таймзоном — упростим.
    if let Some((d, t)) = s.split_once('T') {
        let t = t.split(['+', 'Z']).next().unwrap_or(t);
        return normalize_date(&format!("{d} {t}"));
    }
    chrono_now()
}

fn chrono_now() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
