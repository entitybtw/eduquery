//! Аналитика: сбор контекста об ученике и подготовка промптов для LLM.

use anyhow::Result;

use crate::db::Db;
use crate::students::{get_student, StudentDetail};

/// Человекочитаемый отчёт по ученику — уходит в LLM как контекст.
pub fn build_student_context(db: &Db, student_id: i64) -> Result<String> {
    let detail = get_student(db, student_id)?
        .ok_or_else(|| anyhow::anyhow!("ученик не найден"))?;
    Ok(render_context(&detail))
}

/// Отчёт в компактном текстовом виде.
pub fn render_context(d: &StudentDetail) -> String {
    let s = &d.summary;
    let mut out = String::new();

    out.push_str("== ИДЕНТИФИКАЦИЯ ==\n");
    out.push_str(&format!("ФИО: {}\n", s.full_name));
    out.push_str(&format!("Внутренний ID: {}\n", s.external_id));
    if !s.group_name.is_empty() {
        out.push_str(&format!("Учебная группа: {}\n", s.group_name));
    }
    if !s.email.is_empty() {
        out.push_str(&format!("Email: {}\n", s.email));
    }
    out.push_str(&format!(
        "Курс: {}\n",
        s.grade.map(|g| g.to_string()).unwrap_or_else(|| "не указан".into())
    ));

    out.push_str("\n== ОБЩАЯ СВОДКА ==\n");
    out.push_str(&format!("Количество выполненных работ: {}\n", s.activities));
    if let Some(r) = s.avg_ratio {
        out.push_str(&format!(
            "Средняя успеваемость: {:.0} процентов (средний балл делённый на максимальный балл, умноженный на 100)\n",
            r * 100.0
        ));
    }
    if let Some(sc) = s.avg_score {
        out.push_str(&format!("Средний полученный балл: {:.1} из 100\n", sc));
    }
    if let Some(m) = s.total_minutes {
        out.push_str(&format!(
            "Затраченное на все работы время: {:.0} минут\n",
            m
        ));
    }
    if let Some(l) = &s.last_activity {
        out.push_str(&format!("Последняя активность была: {l}\n"));
    }

    if !d.by_subject.is_empty() {
        out.push_str("\n== РЕЗУЛЬТАТЫ ПО ПРЕДМЕТАМ ==\n");
        for sub in &d.by_subject {
            out.push_str(&format!(
                "Предмет \"{}\": выполнено {} работ, средняя успеваемость {}, время на работы {:.0} минут\n",
                sub.subject,
                sub.count,
                sub.avg_ratio
                    .map(|r| format!("{:.0} процентов", r * 100.0))
                    .unwrap_or_else(|| "нет данных".into()),
                sub.total_minutes.unwrap_or(0.0)
            ));
        }
    }

    if !d.by_kind.is_empty() {
        out.push_str("\n== ТИПЫ АКТИВНОСТЕЙ ==\n");
        for k in &d.by_kind {
            out.push_str(&format!(
                "{}: {} записей, средняя успеваемость {}\n",
                kind_label(&k.kind),
                k.count,
                k.avg_ratio
                    .map(|r| format!("{:.0} процентов", r * 100.0))
                    .unwrap_or_else(|| "нет данных".into())
            ));
        }
    }

    if !d.timeline.is_empty() {
        out.push_str("\n== ДИНАМИКА ПО МЕСЯЦАМ ==\n");
        for t in &d.timeline {
            out.push_str(&format!(
                "Месяц {}: {} активностей, средняя успеваемость {}\n",
                t.period,
                t.count,
                t.avg_ratio
                    .map(|r| format!("{:.0} процентов", r * 100.0))
                    .unwrap_or_else(|| "нет данных".into())
            ));
        }
    }

    if !d.recent.is_empty() {
        out.push_str("\n== ПОСЛЕДНИЕ РАБОТЫ (САМЫЕ СВЕЖИЕ СВЕРХУ) ==\n");
        for a in d.recent.iter().take(25) {
            let score = match (a.score, a.max_score) {
                (Some(sc), Some(m)) if m > 0.0 => {
                    format!("{sc} из {m} (успеваемость {:.0} процентов)", sc / m * 100.0)
                }
                _ => "не оценивалось".to_string(),
            };
            let dur = a
                .duration_min
                .map(|m| format!("{:.0} минут", m))
                .unwrap_or_else(|| "нет данных".to_string());
            out.push_str(&format!(
                "дата {}; работа \"{}\"; предмет \"{}\"; результат {}; статус {}; время {}\n",
                a.occurred_at,
                a.title,
                a.subject,
                score,
                status_label(&a.status),
                dur
            ));
        }
    }

    out
}

/// Человекочитаемое название типа активности.
fn kind_label(kind: &str) -> &'static str {
    match kind {
        "task" => "Домашние работы",
        "quiz" => "Контрольные работы",
        "submission" => "Проектные работы",
        "attendance" => "Занятия (посещаемость)",
        "grade" => "Итоговые оценки",
        _ => "Прочие записи",
    }
}

/// Человекочитаемый статус работы.
fn status_label(status: &str) -> &'static str {
    match status {
        "done" => "выполнено вовремя",
        "late" => "выполнено с опозданием",
        "missing" => "не выполнено",
        "submitted" => "сдано",
        _ => "неизвестный статус",
    }
}

/// Системный промпт по умолчанию.
pub const DEFAULT_SYSTEM_PROMPT: &str = "\
Ты — методист-аналитик в вузе. Тебе дают статистику учебной деятельности одного студента. \
Отвечай на русском языке.

Правила:
- Опирайся ТОЛЬКО на переданные цифры. Не придумывай оценки, события, имена преподавателей и темы, которых нет в данных.
- Если какой-то раздел данных пуст или чего-то не хватает — прямо скажи это в конце ответа.
- Не выдумывай значения: если в строке написано «нет данных», так и напиши.
- Структура ответа: заголовки markdown (##) и нумерованные списки.
- Пиши конкретно: «средний результат 64% по Английскому языку (4 работы)», а не «в целом неплохо».
- НИКОГДА не копируй строки статистики дословно. Перескажи их своими словами и используй цифры в выводах.
- Не перечисляй все работы подряд — выбирай только показательные.";

/// Промпт для полного отчёта об ученике.
pub fn build_report_prompt(context: &str, focus: Option<&str>) -> String {
    let mut p = String::from(
        "Составь развёрнутый разбор учебной деятельности этого студента.\n\n",
    );
    if let Some(f) = focus {
        if !f.trim().is_empty() {
            p.push_str(&format!("ОСОБЫЙ ФОКУС РАЗБОРА: {}\n\n", f.trim()));
        }
    }
    p.push_str("СТАТИСТИКА СТУДЕНТА:\n");
    p.push_str(context);
    p.push_str(
        "\n\nСделай разбор по этим шести пунктам, каждый — отдельным заголовком ##:\n\
1. Краткое резюме — 2-3 предложения с опорой на цифры.\n\
2. Сильные стороны — что конкретно получается, с числами.\n\
3. Проблемные места — где просадка, с числами.\n\
4. Динамика по месяцам — что менялось.\n\
5. Рекомендации преподавателю — 3-5 конкретных действий.\n\
6. Чего не хватает в данных — что стоит уточнить.",
    );
    p
}

/// Промпт для сравнения двух учеников.
pub fn build_compare_prompt(a: &str, b: &str) -> String {
    format!(
        "Сравни двух студентов по их статистике учебной деятельности.\n\n\
=== СТУДЕНТ A ===\n{a}\n\n=== СТУДЕНТ B ===\n{b}\n\n\
Сделай сравнение по пунктам, каждый — заголовком ##:\n\
1. Кто сильнее и насколько — с конкретными процентами.\n\
2. Различия в профиле предметов.\n\
3. Различия в динамике по месяцам.\n\
4. Отдельный блок: что нужно студенту A.\n\
5. Отдельный блок: что нужно студенту B.\n\
6. Общий вывод для преподавателя.",
    )
}

/// Промпт для общего обзора группы/потока.
pub fn build_overview_prompt(summary: &str, groups: &str) -> String {
    format!(
        "На основе сводной статистики по учебному потоку напиши аналитическую записку для преподавателя.\n\n=== ОБЩАЯ СВОДКА ===\n{summary}\n\n=== ПО ГРУППАМ ===\n{groups}\n\n\
Записка должна содержать разделы ##:\n\
1. Общая картина успеваемости — с цифрами.\n\
2. Сравнение групп — какая группа выделяется и почему.\n\
3. Тревожные сигналы — где есть просадки.\n\
4. Приоритетные действия преподавателя — 3-5 пунктов.",
    )
}

/// Промпт «расскажи всё про ученика» — свободная форма.
pub fn build_freeform_prompt(context: &str, question: &str) -> String {
    format!(
        "Ниже — полная статистика учебной деятельности студента. Ответь на вопрос, опираясь только на эти данные. Если ответа в данных нет — так и напиши.\n\n\
СТАТИСТИКА СТУДЕНТА:\n{context}\n\nВОПРОС: {question}"
    )
}

/// Структурированный профиль ученика в JSON — для машинного чтения.
pub fn build_profile_json(d: &StudentDetail) -> serde_json::Value {
    serde_json::json!({
        "id": d.summary.id,
        "external_id": d.summary.external_id,
        "full_name": d.summary.full_name,
        "group": d.summary.group_name,
        "grade": d.summary.grade,
        "activities": d.summary.activities,
        "avg_ratio": d.summary.avg_ratio,
        "total_minutes": d.summary.total_minutes,
        "subjects": d.by_subject,
        "kinds": d.by_kind,
        "timeline": d.timeline,
    })
}
