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
    /// Средняя оценка по 5-балльной шкале.
    pub avg_grade: Option<f64>,
    /// Пропущено уроков всего / из них без уважительной причины.
    pub absences: i64,
    pub absences_unexcused: i64,
    /// Записей в портфолио (достижения, грамоты, благодарности…).
    pub portfolio: i64,
    pub praises: i64,
    pub remarks: i64,
    /// Питание: записей всего / когда питался.
    pub meals: i64,
    pub meals_eaten: i64,
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
    pub grades: Vec<GradeStat>,
    pub grade_items: Vec<Activity>,
    pub absence: AbsenceInfo,
    pub portfolio_items: Vec<Activity>,
    pub behavior: BehaviorInfo,
    pub meals: MealInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct MealTypeStat {
    pub meal: String,
    pub eaten: i64,
    pub skipped: i64,
}

/// Питание: сколько раз питался, по видам (завтрак/обед…), форма оплаты.
#[derive(Debug, Clone, Serialize)]
pub struct MealInfo {
    pub total: i64,
    pub eaten: i64,
    pub skipped: i64,
    /// Форма оплаты (бесплатное / льготное / платное) — по последней записи.
    pub payment: String,
    pub by_type: Vec<MealTypeStat>,
    pub items: Vec<Activity>,
}

/// Оценки по предмету.
#[derive(Debug, Clone, Serialize)]
pub struct GradeStat {
    pub subject: String,
    pub count: i64,
    pub avg_grade: Option<f64>,
    /// Последние оценки, свежие первыми.
    pub last: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReasonStat {
    pub reason: String,
    pub lessons: i64,
}

/// Пропуски: итоги, причины и список.
#[derive(Debug, Clone, Serialize)]
pub struct AbsenceInfo {
    pub total: i64,
    pub excused: i64,
    pub unexcused: i64,
    pub by_reason: Vec<ReasonStat>,
    pub items: Vec<Activity>,
}

/// Поведение: поощрения, замечания и список записей.
#[derive(Debug, Clone, Serialize)]
pub struct BehaviorInfo {
    pub praises: i64,
    pub remarks: i64,
    pub items: Vec<Activity>,
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
    pub reason: String,
    pub category: String,
    pub level: String,
    pub comment: String,
    pub amount: f64,
}

const NON_ACADEMIC: &str = "('absence','portfolio','behavior','meal')";

const ACT_COLS: &str = "id, kind, title, subject, score, max_score, grade, status, \
    duration_min, occurred_at, details, reason, category, level, comment, amount";

/// Общий SELECT для карточки/списка ученика (без WHERE).
const SUMMARY_SELECT: &str = "SELECT s.id, s.external_id, s.full_name, s.group_name, s.email, s.grade,
    (SELECT COUNT(*) FROM activities a WHERE a.student_id = s.id AND a.kind NOT IN ('absence','portfolio','behavior','meal')) AS acts,
    (SELECT AVG(a.score) FROM activities a WHERE a.student_id = s.id AND a.score IS NOT NULL) AS avg_score,
    (SELECT AVG(a.score / a.max_score) FROM activities a
       WHERE a.student_id = s.id AND a.max_score IS NOT NULL AND a.max_score > 0) AS avg_ratio,
    (SELECT SUM(a.duration_min) FROM activities a WHERE a.student_id = s.id) AS total_min,
    (SELECT MAX(a.occurred_at) FROM activities a WHERE a.student_id = s.id) AS last_at,
    (SELECT AVG(a.grade) FROM activities a WHERE a.student_id = s.id AND a.kind = 'grade' AND a.grade IS NOT NULL) AS avg_grade,
    (SELECT CAST(COALESCE(SUM(a.amount),0) AS INTEGER) FROM activities a WHERE a.student_id = s.id AND a.kind = 'absence') AS absn,
    (SELECT CAST(COALESCE(SUM(a.amount),0) AS INTEGER) FROM activities a WHERE a.student_id = s.id AND a.kind = 'absence' AND a.status = 'unexcused') AS absn_bad,
    (SELECT COUNT(*) FROM activities a WHERE a.student_id = s.id AND a.kind = 'portfolio') AS portf,
    (SELECT COUNT(*) FROM activities a WHERE a.student_id = s.id AND a.kind = 'behavior' AND a.category = 'Поощрение') AS praises,
    (SELECT COUNT(*) FROM activities a WHERE a.student_id = s.id AND a.kind = 'behavior' AND a.category IN ('Замечание','Нарушение')) AS remarks,
    (SELECT COUNT(*) FROM activities a WHERE a.student_id = s.id AND a.kind = 'meal') AS meals,
    (SELECT COUNT(*) FROM activities a WHERE a.student_id = s.id AND a.kind = 'meal' AND a.status = 'ate') AS meals_eaten
    FROM students s";

fn map_summary(r: &Row<'_>) -> rusqlite::Result<StudentSummary> {
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
        avg_grade: r.get(11)?,
        absences: r.get(12)?,
        absences_unexcused: r.get(13)?,
        portfolio: r.get(14)?,
        praises: r.get(15)?,
        remarks: r.get(16)?,
        meals: r.get(17)?,
        meals_eaten: r.get(18)?,
    })
}

/// Ученик по идентификатору БД.
pub fn get_student(db: &Db, id: i64) -> Result<Option<StudentDetail>> {
    let conn = db.conn();
    let summary: Option<StudentSummary> = conn
        .query_row(
            &format!("{SUMMARY_SELECT} WHERE s.id = ?1"),
            params![id],
            map_summary,
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
             WHERE student_id = ?1 AND subject <> '' AND kind NOT IN ('absence','portfolio','behavior','meal')
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
             FROM activities WHERE student_id = ?1 AND kind NOT IN ('absence','portfolio','behavior','meal')
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

    let load = |kind_filter: &str, limit: i64| -> Result<Vec<Activity>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {ACT_COLS} FROM activities WHERE student_id = ?1 AND {kind_filter}
             ORDER BY occurred_at DESC, id DESC LIMIT {limit}"
        ))?;
        let rows = stmt.query_map(params![id], map_activity)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    };
    let recent = load(&format!("kind NOT IN {NON_ACADEMIC}"), 60)?;
    let grade_items = load("kind = 'grade' AND grade IS NOT NULL", 200)?;
    let absence_items = load("kind = 'absence'", 200)?;
    let portfolio_items = load("kind = 'portfolio'", 200)?;
    let behavior_items = load("kind = 'behavior'", 200)?;
    let meal_items = load("kind = 'meal'", 200)?;

    // Оценки по предметам.
    let mut grades: Vec<GradeStat> = Vec::new();
    for g in &grade_items {
        let subj = if g.subject.is_empty() { "Без предмета".to_string() } else { g.subject.clone() };
        let v = g.grade.unwrap_or(0);
        match grades.iter_mut().find(|x| x.subject == subj) {
            Some(x) => {
                x.count += 1;
                x.last.push(v);
            }
            None => grades.push(GradeStat { subject: subj, count: 1, avg_grade: None, last: vec![v] }),
        }
    }
    for g in &mut grades {
        g.avg_grade = Some(g.last.iter().sum::<i64>() as f64 / g.last.len().max(1) as f64);
        g.last.truncate(6);
    }
    grades.sort_by(|a, b| a.subject.cmp(&b.subject));

    // Пропуски.
    let absence = {
        let lessons = |it: &Activity| it.amount.round() as i64;
        let total: i64 = absence_items.iter().map(lessons).sum();
        let unexcused: i64 = absence_items.iter().filter(|a| a.status == "unexcused").map(lessons).sum();
        let mut by_reason: Vec<ReasonStat> = Vec::new();
        for a in &absence_items {
            let r = if a.reason.trim().is_empty() { "не указана".to_string() } else { a.reason.clone() };
            match by_reason.iter_mut().find(|x| x.reason == r) {
                Some(x) => x.lessons += lessons(a),
                None => by_reason.push(ReasonStat { reason: r, lessons: lessons(a) }),
            }
        }
        by_reason.sort_by(|a, b| b.lessons.cmp(&a.lessons));
        AbsenceInfo { total, excused: total - unexcused, unexcused, by_reason, items: absence_items }
    };

    let behavior = BehaviorInfo {
        praises: behavior_items.iter().filter(|b| b.category == "Поощрение").count() as i64,
        remarks: behavior_items
            .iter()
            .filter(|b| b.category == "Замечание" || b.category == "Нарушение")
            .count() as i64,
        items: behavior_items,
    };

    let meals = {
        let eaten = meal_items.iter().filter(|m| m.status == "ate").count() as i64;
        let mut by_type: Vec<MealTypeStat> = Vec::new();
        for m in &meal_items {
            let t = if m.category.is_empty() { "Питание".to_string() } else { m.category.clone() };
            let ate = m.status == "ate";
            match by_type.iter_mut().find(|x| x.meal == t) {
                Some(x) => if ate { x.eaten += 1 } else { x.skipped += 1 },
                None => by_type.push(MealTypeStat { meal: t, eaten: ate as i64, skipped: !ate as i64 }),
            }
        }
        MealInfo {
            total: meal_items.len() as i64,
            eaten,
            skipped: meal_items.len() as i64 - eaten,
            payment: meal_items.iter().map(|m| m.level.clone()).find(|l| !l.is_empty()).unwrap_or_default(),
            by_type,
            items: meal_items,
        }
    };

    let mut detail = StudentDetail {
        summary,
        by_subject,
        by_kind,
        timeline,
        recent,
        strengths: Vec::new(),
        weaknesses: Vec::new(),
        grades,
        grade_items,
        absence,
        portfolio_items,
        behavior,
        meals,
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
    if let Some(g) = detail.summary.avg_grade {
        if g >= 4.5 {
            detail.strengths.push(format!("Высокая средняя оценка ({g:.1} из 5)"));
        } else if g < 3.5 {
            detail.weaknesses.push(format!("Низкая средняя оценка ({g:.1} из 5)"));
        }
    }
    if detail.absence.unexcused >= 5 {
        detail.weaknesses.push(format!(
            "Много пропусков без уважительной причины: {} ур.",
            detail.absence.unexcused
        ));
    }
    if detail.summary.portfolio >= 3 {
        detail.strengths.push(format!("Богатое портфолио: {} записей", detail.summary.portfolio));
    }
    if detail.behavior.remarks >= 3 {
        detail.weaknesses.push(format!("Замечаний по поведению: {}", detail.behavior.remarks));
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
        reason: r.get(11)?,
        category: r.get(12)?,
        level: r.get(13)?,
        comment: r.get(14)?,
        amount: r.get(15)?,
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
    let mut sql = format!("{SUMMARY_SELECT} WHERE 1=1");

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
    let rows = stmt.query_map(params_from_iter(args.iter()), map_summary)?;
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
    pub avg_grade: Option<f64>,
    pub absences: i64,
    pub absences_unexcused: i64,
    pub portfolio: i64,
    pub remarks: i64,
    pub praises: i64,
    pub meals: i64,
    pub meals_eaten: i64,
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
    let activities: i64 = conn.query_row(
        "SELECT COUNT(*) FROM activities WHERE kind NOT IN ('absence','portfolio','behavior','meal')",
        [],
        |r| r.get(0),
    )?;
    let avg_grade: Option<f64> = conn.query_row(
        "SELECT AVG(grade) FROM activities WHERE kind = 'grade' AND grade IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    let sum_i = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0) };
    let absences = sum_i("SELECT CAST(COALESCE(SUM(amount),0) AS INTEGER) FROM activities WHERE kind = 'absence'");
    let absences_unexcused = sum_i("SELECT CAST(COALESCE(SUM(amount),0) AS INTEGER) FROM activities WHERE kind = 'absence' AND status = 'unexcused'");
    let portfolio = sum_i("SELECT COUNT(*) FROM activities WHERE kind = 'portfolio'");
    let praises = sum_i("SELECT COUNT(*) FROM activities WHERE kind = 'behavior' AND category = 'Поощрение'");
    let remarks = sum_i("SELECT COUNT(*) FROM activities WHERE kind = 'behavior' AND category IN ('Замечание','Нарушение')");
    let meals = sum_i("SELECT COUNT(*) FROM activities WHERE kind = 'meal'");
    let meals_eaten = sum_i("SELECT COUNT(*) FROM activities WHERE kind = 'meal' AND status = 'ate'");
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
        avg_grade,
        absences,
        absences_unexcused,
        portfolio,
        remarks,
        praises,
        meals,
        meals_eaten,
    })
}

// ───────────────────────────── Статистика ─────────────────────────────

/// Агрегаты для вкладки «Статистика»; `group` пустая строка — все группы.
pub fn stats(db: &Db, group: &str) -> Result<Value> {
    use serde_json::json;
    let students = list_students(db, "", group, "")?;
    let conn = db.conn();
    // Общая выборка: активности учеников выбранной группы.
    const FROM: &str = "FROM activities a JOIN students s ON s.id = a.student_id \
                        WHERE (?1 = '' OR s.group_name = ?1)";

    let rows = |sql: String, map: &dyn Fn(&Row<'_>) -> rusqlite::Result<Value>| -> Result<Vec<Value>> {
        let mut stmt = conn.prepare(&sql)?;
        let it = stmt.query_map(params![group], |r| map(r))?;
        Ok(it.collect::<rusqlite::Result<Vec<_>>>()?)
    };

    let grade_dist = rows(
        format!("SELECT a.grade, COUNT(*) {FROM} AND a.kind = 'grade' AND a.grade BETWEEN 1 AND 5 GROUP BY a.grade ORDER BY a.grade"),
        &|r| Ok(json!({"grade": r.get::<_, i64>(0)?, "count": r.get::<_, i64>(1)?})),
    )?;
    let grade_by_subject = rows(
        format!("SELECT a.subject, AVG(a.grade), COUNT(*) {FROM} AND a.kind = 'grade' AND a.grade IS NOT NULL AND a.subject <> '' GROUP BY a.subject ORDER BY AVG(a.grade) DESC"),
        &|r| Ok(json!({"subject": r.get::<_, String>(0)?, "avg_grade": r.get::<_, f64>(1)?, "count": r.get::<_, i64>(2)?})),
    )?;
    let absences_by_reason = rows(
        format!("SELECT CASE WHEN a.reason = '' THEN 'не указана' ELSE a.reason END, CAST(SUM(a.amount) AS INTEGER) {FROM} AND a.kind = 'absence' GROUP BY 1 ORDER BY 2 DESC"),
        &|r| Ok(json!({"reason": r.get::<_, String>(0)?, "lessons": r.get::<_, i64>(1)?})),
    )?;
    let absences_by_month = rows(
        format!("SELECT substr(a.occurred_at,1,7), CAST(SUM(CASE WHEN a.status='unexcused' THEN 0 ELSE a.amount END) AS INTEGER), CAST(SUM(CASE WHEN a.status='unexcused' THEN a.amount ELSE 0 END) AS INTEGER) {FROM} AND a.kind = 'absence' GROUP BY 1 ORDER BY 1"),
        &|r| Ok(json!({"period": r.get::<_, String>(0)?, "excused": r.get::<_, i64>(1)?, "unexcused": r.get::<_, i64>(2)?})),
    )?;
    let portfolio_by_category = rows(
        format!("SELECT CASE WHEN a.category = '' THEN 'Без категории' ELSE a.category END, COUNT(*) {FROM} AND a.kind = 'portfolio' GROUP BY 1 ORDER BY 2 DESC"),
        &|r| Ok(json!({"category": r.get::<_, String>(0)?, "count": r.get::<_, i64>(1)?})),
    )?;
    let portfolio_by_level = rows(
        format!("SELECT CASE WHEN a.level = '' THEN 'Не указан' ELSE a.level END, COUNT(*) {FROM} AND a.kind = 'portfolio' GROUP BY 1 ORDER BY 2 DESC"),
        &|r| Ok(json!({"level": r.get::<_, String>(0)?, "count": r.get::<_, i64>(1)?})),
    )?;
    let meals_by_type = rows(
        format!("SELECT CASE WHEN a.category = '' THEN 'Питание' ELSE a.category END, SUM(a.status = 'ate'), SUM(a.status <> 'ate') {FROM} AND a.kind = 'meal' GROUP BY 1 ORDER BY 2 + 3 DESC"),
        &|r| Ok(json!({"meal": r.get::<_, String>(0)?, "eaten": r.get::<_, i64>(1)?, "skipped": r.get::<_, i64>(2)?})),
    )?;
    let meals_by_payment = rows(
        format!("SELECT CASE WHEN a.level = '' THEN 'Не указана' ELSE a.level END, SUM(a.status = 'ate'), SUM(a.status <> 'ate') {FROM} AND a.kind = 'meal' GROUP BY 1 ORDER BY 2 + 3 DESC"),
        &|r| Ok(json!({"payment": r.get::<_, String>(0)?, "eaten": r.get::<_, i64>(1)?, "skipped": r.get::<_, i64>(2)?})),
    )?;
    let behavior_by_month = rows(
        format!("SELECT substr(a.occurred_at,1,7), SUM(a.category = 'Поощрение'), SUM(a.category = 'Замечание'), SUM(a.category = 'Нарушение') {FROM} AND a.kind = 'behavior' GROUP BY 1 ORDER BY 1"),
        &|r| Ok(json!({"period": r.get::<_, String>(0)?, "praise": r.get::<_, i64>(1)?, "remark": r.get::<_, i64>(2)?, "violation": r.get::<_, i64>(3)?})),
    )?;
    let by_month = rows(
        format!("SELECT substr(a.occurred_at,1,7), AVG(CASE WHEN a.max_score > 0 THEN a.score / a.max_score END), AVG(CASE WHEN a.kind='grade' THEN a.grade END) {FROM} AND a.kind NOT IN ('absence','portfolio','behavior','meal') GROUP BY 1 ORDER BY 1"),
        &|r| Ok(json!({"period": r.get::<_, String>(0)?, "avg_ratio": r.get::<_, Option<f64>>(1)?, "avg_grade": r.get::<_, Option<f64>>(2)?})),
    )?;
    drop(conn);

    // По группам (без фильтра по группе — сравнение групп между собой).
    let all = list_students(db, "", "", "")?;
    let mut groups: Vec<Value> = Vec::new();
    let mut names: Vec<String> = all.iter().map(|s| s.group_name.clone()).filter(|g| !g.is_empty()).collect();
    names.sort();
    names.dedup();
    for g in names {
        let m: Vec<&StudentSummary> = all.iter().filter(|s| s.group_name == g).collect();
        let n = m.len() as f64;
        let grades: Vec<f64> = m.iter().filter_map(|s| s.avg_grade).collect();
        let ratios: Vec<f64> = m.iter().filter_map(|s| s.avg_ratio).collect();
        let avg = |v: &Vec<f64>| if v.is_empty() { Value::Null } else { json!(v.iter().sum::<f64>() / v.len() as f64) };
        groups.push(json!({
            "group": g,
            "students": m.len(),
            "avg_grade": avg(&grades),
            "avg_ratio": avg(&ratios),
            "absences": m.iter().map(|s| s.absences).sum::<i64>(),
            "absences_unexcused": m.iter().map(|s| s.absences_unexcused).sum::<i64>(),
            "absences_per_student": m.iter().map(|s| s.absences).sum::<i64>() as f64 / n.max(1.0),
            "portfolio": m.iter().map(|s| s.portfolio).sum::<i64>(),
            "praises": m.iter().map(|s| s.praises).sum::<i64>(),
            "remarks": m.iter().map(|s| s.remarks).sum::<i64>(),
            "meals": m.iter().map(|s| s.meals).sum::<i64>(),
            "meals_eaten": m.iter().map(|s| s.meals_eaten).sum::<i64>(),
        }));
    }

    let grades: Vec<f64> = students.iter().filter_map(|s| s.avg_grade).collect();
    let totals = json!({
        "students": students.len(),
        "avg_grade": if grades.is_empty() { Value::Null } else { json!(grades.iter().sum::<f64>() / grades.len() as f64) },
        "absences": students.iter().map(|s| s.absences).sum::<i64>(),
        "absences_unexcused": students.iter().map(|s| s.absences_unexcused).sum::<i64>(),
        "portfolio": students.iter().map(|s| s.portfolio).sum::<i64>(),
        "praises": students.iter().map(|s| s.praises).sum::<i64>(),
        "remarks": students.iter().map(|s| s.remarks).sum::<i64>(),
        "meals": students.iter().map(|s| s.meals).sum::<i64>(),
        "meals_eaten": students.iter().map(|s| s.meals_eaten).sum::<i64>(),
    });

    Ok(json!({
        "group": group,
        "totals": totals,
        "grade_dist": grade_dist,
        "grade_by_subject": grade_by_subject,
        "absences_by_reason": absences_by_reason,
        "absences_by_month": absences_by_month,
        "portfolio_by_category": portfolio_by_category,
        "portfolio_by_level": portfolio_by_level,
        "behavior_by_month": behavior_by_month,
        "meals_by_type": meals_by_type,
        "meals_by_payment": meals_by_payment,
        "by_month": by_month,
        "by_group": groups,
        "students": students,
    }))
}

// ───────────────────────────── Импорт данных ─────────────────────────────

/// Одна запись, приходящая из импорта. Заголовки колонок приводятся к виду
/// `макс_балл` (см. `labels::norm_header`), поэтому подходят и русские, и английские названия.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ImportRow {
    #[serde(default, alias = "student_id", alias = "id", alias = "id_ученика", alias = "ид_ученика", alias = "внешний_id", alias = "идентификатор", alias = "табельный_номер")]
    pub external_id: Option<String>,
    #[serde(default, alias = "name", alias = "фио", alias = "ученик", alias = "имя")]
    pub full_name: Option<String>,
    #[serde(default, alias = "group", alias = "группа", alias = "class", alias = "класс")]
    pub group_name: Option<String>,
    #[serde(default, alias = "email", alias = "эл_почта", alias = "почта", alias = "e_mail", alias = "электронная_почта")]
    pub email: Option<String>,
    #[serde(default, alias = "subject", alias = "course", alias = "предмет", alias = "дисциплина")]
    pub subject: Option<String>,
    #[serde(default, alias = "title", alias = "task", alias = "название", alias = "задание", alias = "тема", alias = "наименование")]
    pub title: Option<String>,
    #[serde(default, alias = "kind", alias = "type", alias = "тип", alias = "тип_записи")]
    pub kind: Option<String>,
    #[serde(default, alias = "score", alias = "балл", alias = "баллы", alias = "результат", deserialize_with = "de_opt_f64")]
    pub score: Option<f64>,
    #[serde(default, alias = "max_score", alias = "макс_балл", alias = "max", alias = "максимальный_балл", deserialize_with = "de_opt_f64")]
    pub max_score: Option<f64>,
    #[serde(default, alias = "grade", alias = "оценка", alias = "отметка", deserialize_with = "de_opt_i64")]
    pub grade: Option<i64>,
    #[serde(default, alias = "status", alias = "статус", alias = "статус_работы")]
    pub status: Option<String>,
    #[serde(default, alias = "duration", alias = "duration_min", alias = "время_мин", alias = "время", alias = "длительность", alias = "minutes", deserialize_with = "de_opt_f64")]
    pub duration_min: Option<f64>,
    #[serde(default, alias = "date", alias = "дата", alias = "occurred_at", alias = "timestamp", alias = "дата_записи")]
    pub occurred_at: Option<String>,
    /// Пропуски: сколько уроков.
    #[serde(default, alias = "amount", alias = "пропущено_уроков", alias = "уроков", alias = "кол_во_уроков", alias = "количество", deserialize_with = "de_opt_f64")]
    pub amount: Option<f64>,
    #[serde(default, alias = "reason", alias = "причина", alias = "причина_пропуска")]
    pub reason: Option<String>,
    /// «да/нет» — уважительная ли причина пропуска.
    #[serde(default, alias = "excused", alias = "уважительная", alias = "уважительный", alias = "уважительная_причина", deserialize_with = "de_opt_string")]
    pub excused: Option<String>,
    #[serde(default, alias = "category", alias = "категория", alias = "вид")]
    pub category: Option<String>,
    #[serde(default, alias = "level", alias = "уровень", alias = "уровень_оплата", alias = "оплата", alias = "форма_питания", alias = "форма_оплаты")]
    pub level: Option<String>,
    #[serde(default, alias = "comment", alias = "комментарий", alias = "описание", alias = "примечание")]
    pub comment: Option<String>,
}

fn de_opt_f64<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<f64>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => {
            let t = s.trim().replace(',', ".");
            if t.is_empty() { None } else { t.parse().ok() }
        }
        _ => None,
    })
}

fn de_opt_i64<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<i64>, D::Error> {
    de_opt_f64(d).map(|v| v.map(|x| x.round() as i64))
}

/// Строка или bool/число из JSON → строка.
fn de_opt_string<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<String>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::String(s)) => Some(s),
        Some(Value::Bool(b)) => Some(b.to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    })
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

impl ImportResult {
    fn new() -> Self {
        Self { rows_read: 0, students_created: 0, students_updated: 0, activities_created: 0, errors: Vec::new() }
    }
}

/// Импортировать CSV (разделитель «,» или «;», заголовки по-русски или по-английски).
pub fn import_csv(db: &Db, content: &str) -> Result<ImportResult> {
    let content = content.trim_start_matches('\u{feff}');
    // Excel в русской локали сохраняет CSV с «;».
    let first = content.lines().next().unwrap_or("");
    let delim = if first.matches(';').count() > first.matches(',').count() { b';' } else { b',' };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delim)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(content.as_bytes());

    let headers = reader
        .headers()
        .context("не удалось прочитать заголовки CSV")?
        .clone();

    let mut result = ImportResult::new();

    for (idx, rec) in reader.records().enumerate() {
        let rec = match rec {
            Ok(r) => r,
            Err(e) => {
                result.errors.push(format!("строка {}: {e}", idx + 2));
                continue;
            }
        };
        if rec.iter().all(|v| v.trim().is_empty()) {
            continue; // пустая строка
        }
        result.rows_read += 1;

        let mut map = serde_json::Map::new();
        for (h, v) in headers.iter().zip(rec.iter()) {
            map.insert(crate::labels::norm_header(h), Value::String(v.to_string()));
        }
        let row: ImportRow = match serde_json::from_value(Value::Object(map)) {
            Ok(r) => r,
            Err(e) => {
                result.errors.push(format!("строка {}: не разобрана ({e})", idx + 2));
                continue;
            }
        };
        import_one(db, &row, idx + 2, &mut result);
    }

    Ok(result)
}

/// Импортировать JSON-массив объектов (ключи по-русски или по-английски).
pub fn import_json(db: &Db, text: &str) -> Result<ImportResult> {
    let items: Vec<serde_json::Map<String, Value>> =
        serde_json::from_str(text).context("не удалось разобрать JSON (нужен массив объектов)")?;
    let mut result = ImportResult::new();
    for (i, obj) in items.into_iter().enumerate() {
        result.rows_read += 1;
        let map: serde_json::Map<String, Value> = obj
            .into_iter()
            .map(|(k, v)| (crate::labels::norm_header(&k), v))
            .collect();
        match serde_json::from_value::<ImportRow>(Value::Object(map)) {
            Ok(row) => import_one(db, &row, i + 1, &mut result),
            Err(e) => result.errors.push(format!("запись {}: не разобрана ({e})", i + 1)),
        }
    }
    Ok(result)
}

fn import_one(db: &Db, row: &ImportRow, line: usize, result: &mut ImportResult) {
    match upsert_student(db, row, result) {
        Ok(Some(student_id)) => match insert_activity(db, student_id, row) {
            Ok(()) => result.activities_created += 1,
            Err(e) => result.errors.push(format!("строка {line}: {e}")),
        },
        Ok(None) => result.errors.push(format!(
            "строка {line}: не удалось определить ученика (нужны «ID ученика» или «ФИО»)"
        )),
        Err(e) => result.errors.push(format!("строка {line}: {e}")),
    }
}

/// Добавить запись существующему ученику (форма в карточке).
pub fn add_record(db: &Db, student_id: i64, row: &ImportRow) -> Result<()> {
    insert_activity(db, student_id, row)
}

/// Удалить запись.
pub fn delete_record(db: &Db, id: i64) -> Result<bool> {
    Ok(db.conn().execute("DELETE FROM activities WHERE id = ?1", params![id])? > 0)
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
    use crate::labels;
    let conn = db.conn();
    let has = |o: &Option<String>| o.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false);

    // Тип записи: явный, иначе выводим из заполненных полей.
    let kind = match row.kind.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(k) => labels::kind_from(k),
        None if row.grade.is_some() && row.score.is_none() => "grade".to_string(),
        None if has(&row.reason) => "absence".to_string(),
        None => "task".to_string(),
    };

    let subject = row.subject.clone().unwrap_or_default().trim().to_string();
    let category = labels::norm_category(&kind, row.category.as_deref().unwrap_or(""));
    let level = row.level.clone().unwrap_or_default().trim().to_string();
    let comment = row.comment.clone().unwrap_or_default().trim().to_string();
    let reason = row.reason.clone().unwrap_or_default().trim().to_string();

    let title = match row.title.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(t) => t.to_string(),
        None => match kind.as_str() {
            "absence" => "Пропуск занятия".to_string(),
            "portfolio" if !category.is_empty() => category.clone(),
            "portfolio" => "Достижение".to_string(),
            "behavior" if !category.is_empty() => category.clone(),
            "behavior" => "Запись о поведении".to_string(),
            "meal" if !category.is_empty() => category.clone(),
            "meal" => "Питание".to_string(),
            "grade" => format!("Оценка: {}", if subject.is_empty() { "без предмета" } else { &subject }),
            k => labels::kind_ru(k).to_string(),
        },
    };

    // Баллы — только для учебных работ; у пропусков/портфолио/поведения их нет.
    let academic = !matches!(kind.as_str(), "absence" | "portfolio" | "behavior" | "meal");
    let max_score = row.max_score.filter(|m| *m > 0.0).filter(|_| academic);
    let score = row.score.filter(|_| max_score.is_some());
    let grade = row.grade.filter(|g| (1..=5).contains(g));

    let occurred_at = match row.occurred_at.as_ref().map(|s| s.trim()) {
        Some(s) if !s.is_empty() => normalize_date(s),
        _ => chrono_now(),
    };

    // Статус: у пропуска — уважительный/неуважительный.
    let (status, amount) = if kind == "absence" {
        let excused = row
            .excused
            .as_deref()
            .and_then(labels::parse_bool)
            .or_else(|| {
                row.status.as_deref().map(labels::status_from).and_then(|s| match s.as_str() {
                    "excused" => Some(true),
                    "unexcused" => Some(false),
                    _ => None,
                })
            })
            .unwrap_or_else(|| labels::reason_is_excused(&reason));
        (
            if excused { "excused" } else { "unexcused" }.to_string(),
            row.amount.filter(|a| *a > 0.0).unwrap_or(1.0),
        )
    } else {
        (
            row.status
                .as_deref()
                .map(labels::status_from)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| if kind == "meal" { "ate" } else { "done" }.to_string()),
            1.0,
        )
    };

    conn.execute(
        "INSERT INTO activities(student_id, kind, title, subject, score, max_score,
                                grade, status, duration_min, occurred_at, details,
                                reason, category, level, comment, amount)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '{}', ?11, ?12, ?13, ?14, ?15)",
        params![
            student_id,
            kind,
            title,
            subject,
            score,
            max_score,
            grade,
            status,
            row.duration_min,
            occurred_at,
            reason,
            category,
            level,
            comment,
            amount
        ],
    )?;
    Ok(())
}

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

/// Удалить все данные об учениках (для режима «заменить»).
pub fn clear_all(db: &Db) -> Result<()> {
    let conn = db.conn();
    conn.execute_batch(
        "DELETE FROM analyses; DELETE FROM activities; DELETE FROM students;",
    )?;
    Ok(())
}
