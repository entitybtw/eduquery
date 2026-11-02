//! Аналитика: сбор контекста об ученике и подготовка промптов для LLM.

use anyhow::Result;

use crate::db::Db;
use crate::labels;
use crate::students::{get_student, Activity, StudentDetail};

/// Человекочитаемый отчёт по ученику — уходит в LLM как контекст.
pub fn build_student_context(db: &Db, student_id: i64) -> Result<String> {
    let detail = get_student(db, student_id)?
        .ok_or_else(|| anyhow::anyhow!("ученик не найден"))?;
    Ok(render_context(&detail))
}

fn pct(r: Option<f64>) -> String {
    r.map(|r| format!("{:.0} процентов", r * 100.0))
        .unwrap_or_else(|| "нет данных".into())
}

fn day(s: &str) -> &str {
    s.get(..10).unwrap_or(s)
}

/// Отчёт в компактном текстовом виде. Каждый раздел присутствует всегда:
/// если данных нет, так и написано — модель не должна ничего додумывать.
pub fn render_context(d: &StudentDetail) -> String {
    let s = &d.summary;
    let mut out = String::new();

    out.push_str("== ИДЕНТИФИКАЦИЯ ==\n");
    out.push_str(&format!("ФИО: {}\n", s.full_name));
    out.push_str(&format!("Внутренний ID: {}\n", s.external_id));
    if !s.group_name.is_empty() {
        out.push_str(&format!("Класс/группа: {}\n", s.group_name));
    }
    if let Some(g) = s.grade {
        out.push_str(&format!("Курс: {g}\n"));
    }

    // ── успеваемость ──
    out.push_str("\n== УСПЕВАЕМОСТЬ (РАБОТЫ) ==\n");
    out.push_str(&format!("Количество учебных записей: {}\n", s.activities));
    match s.avg_ratio {
        Some(r) => out.push_str(&format!(
            "Средний результат за работы: {:.0} процентов от максимального балла\n",
            r * 100.0
        )),
        None => out.push_str("Оцениваемых работ с баллами нет.\n"),
    }
    if let Some(m) = s.total_minutes {
        out.push_str(&format!("Время, затраченное на работы: {m:.0} минут\n"));
    }
    if let Some(l) = &s.last_activity {
        out.push_str(&format!("Последняя запись в журнале: {}\n", day(l)));
    }

    if !d.by_subject.is_empty() {
        out.push_str("\nПо предметам:\n");
        for sub in &d.by_subject {
            out.push_str(&format!(
                "- {}: работ {}, средний результат {}, время {:.0} минут\n",
                sub.subject,
                sub.count,
                pct(sub.avg_ratio),
                sub.total_minutes.unwrap_or(0.0)
            ));
        }
    }

    if !d.timeline.is_empty() {
        out.push_str("\nДинамика по месяцам (работы):\n");
        for t in &d.timeline {
            out.push_str(&format!(
                "- {}: записей {}, средний результат {}\n",
                t.period,
                t.count,
                pct(t.avg_ratio)
            ));
        }
    }

    // ── оценки ──
    out.push_str("\n== ОЦЕНКИ (5-БАЛЛЬНАЯ ШКАЛА) ==\n");
    if d.grade_items.is_empty() {
        out.push_str("Оценок в журнале нет.\n");
    } else {
        if let Some(g) = s.avg_grade {
            out.push_str(&format!("Средняя оценка: {g:.2} (оценок всего: {})\n", d.grade_items.len()));
        }
        for g in &d.grades {
            let last = g.last.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
            out.push_str(&format!(
                "- {}: средняя {:.2}, оценок {}, последние (свежие первыми): {}\n",
                g.subject,
                g.avg_grade.unwrap_or(0.0),
                g.count,
                last
            ));
        }
        let low: Vec<_> = d.grades.iter().filter(|g| g.avg_grade.unwrap_or(5.0) < 3.5).collect();
        if !low.is_empty() {
            out.push_str(&format!(
                "Предметы со средней оценкой ниже 3,5: {}\n",
                low.iter().map(|g| g.subject.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
    }

    // ── пропуски ──
    out.push_str("\n== ПРОПУСКИ ==\n");
    if d.absence.items.is_empty() {
        out.push_str("Пропусков в журнале нет.\n");
    } else {
        let a = &d.absence;
        out.push_str(&format!(
            "Пропущено уроков всего: {} (по уважительной причине: {}, без уважительной причины: {})\n",
            a.total, a.excused, a.unexcused
        ));
        out.push_str("Причины пропусков:\n");
        for r in &a.by_reason {
            out.push_str(&format!("- {}: {} ур.\n", r.reason, r.lessons));
        }
        out.push_str("Последние пропуски:\n");
        for it in a.items.iter().take(10) {
            out.push_str(&format!("- {}\n", absence_line(it)));
        }
    }

    // ── портфолио ──
    out.push_str("\n== ПОРТФОЛИО (ДОСТИЖЕНИЯ, ГРАМОТЫ, БЛАГОДАРНОСТИ) ==\n");
    if d.portfolio_items.is_empty() {
        out.push_str("Записей в портфолио нет.\n");
    } else {
        out.push_str(&format!("Всего записей: {}\n", d.portfolio_items.len()));
        let mut cats: Vec<(String, usize)> = Vec::new();
        for it in &d.portfolio_items {
            let c = if it.category.is_empty() { "Без категории".to_string() } else { it.category.clone() };
            match cats.iter_mut().find(|x| x.0 == c) {
                Some(x) => x.1 += 1,
                None => cats.push((c, 1)),
            }
        }
        out.push_str(&format!(
            "По видам: {}\n",
            cats.iter().map(|(c, n)| format!("{c} — {n}")).collect::<Vec<_>>().join("; ")
        ));
        for it in d.portfolio_items.iter().take(12) {
            out.push_str(&format!("- {}\n", portfolio_line(it)));
        }
    }

    // ── поведение ──
    out.push_str("\n== ПОВЕДЕНИЕ ==\n");
    if d.behavior.items.is_empty() {
        out.push_str("Записей о поведении нет.\n");
    } else {
        out.push_str(&format!(
            "Поощрений: {}; замечаний и нарушений: {}; всего записей: {}\n",
            d.behavior.praises,
            d.behavior.remarks,
            d.behavior.items.len()
        ));
        for it in d.behavior.items.iter().take(12) {
            out.push_str(&format!("- {}\n", behavior_line(it)));
        }
    }

    // ── питание ──
    out.push_str("\n== ПИТАНИЕ (ШКОЛЬНАЯ СТОЛОВАЯ) ==\n");
    let m = &d.meals;
    if m.total == 0 {
        out.push_str("Данных о питании нет.\n");
    } else {
        out.push_str(&format!(
            "Записей: {}; питался: {} ({:.0} процентов дней), не питался: {}\n",
            m.total,
            m.eaten,
            m.eaten as f64 / m.total as f64 * 100.0,
            m.skipped
        ));
        if !m.payment.is_empty() {
            out.push_str(&format!("Форма питания: {}\n", m.payment.to_lowercase()));
        }
        for t in &m.by_type {
            out.push_str(&format!("- {}: питался {}, не питался {}\n", t.meal, t.eaten, t.skipped));
        }
    }

    // ── последние работы ──
    if !d.recent.is_empty() {
        out.push_str("\n== ПОСЛЕДНИЕ РАБОТЫ (СВЕЖИЕ СВЕРХУ) ==\n");
        for a in d.recent.iter().take(25) {
            let score = match (a.score, a.max_score) {
                (Some(sc), Some(m)) if m > 0.0 => {
                    format!("{sc} из {m} (результат {:.0} процентов)", sc / m * 100.0)
                }
                _ => match a.grade {
                    Some(g) => format!("оценка {g}"),
                    None => "не оценивалось".to_string(),
                },
            };
            let dur = a
                .duration_min
                .map(|m| format!("{m:.0} минут"))
                .unwrap_or_else(|| "нет данных".to_string());
            out.push_str(&format!(
                "дата {}; {} \"{}\"; предмет \"{}\"; результат {}; статус {}; время {}\n",
                day(&a.occurred_at),
                labels::kind_ru(&a.kind).to_lowercase(),
                a.title,
                a.subject,
                score,
                labels::status_ru(&a.status),
                dur
            ));
        }
    }

    out
}

fn absence_line(a: &Activity) -> String {
    let mut t = format!("{}: {} ур.", day(&a.occurred_at), a.amount.round() as i64);
    if !a.subject.is_empty() {
        t.push_str(&format!(", предмет «{}»", a.subject));
    }
    t.push_str(&format!(
        ", причина: {}, {}",
        if a.reason.is_empty() { "не указана" } else { &a.reason },
        if a.status == "excused" { "уважительная" } else { "без уважительной причины" }
    ));
    if !a.comment.is_empty() {
        t.push_str(&format!(" ({})", a.comment));
    }
    t
}

fn portfolio_line(a: &Activity) -> String {
    let mut t = format!("{}: ", day(&a.occurred_at));
    t.push_str(if a.category.is_empty() { "достижение" } else { &a.category });
    if !a.level.is_empty() {
        t.push_str(&format!(", уровень: {}", a.level.to_lowercase()));
    }
    t.push_str(&format!(" — «{}»", a.title));
    if !a.comment.is_empty() {
        t.push_str(&format!(" ({})", a.comment));
    }
    t
}

fn behavior_line(a: &Activity) -> String {
    let mut t = format!(
        "{}: {}",
        day(&a.occurred_at),
        if a.category.is_empty() { "запись" } else { &a.category }
    );
    if !a.comment.is_empty() {
        t.push_str(&format!(" — {}", a.comment));
    } else if !a.title.is_empty() && a.title != a.category {
        t.push_str(&format!(" — {}", a.title));
    }
    t
}

/// Системный промпт по умолчанию. Это именно инструкция для модели (правила работы с
/// данными и формат ответа), а не просто название роли; редактируется на вкладке «Настройки».
pub const DEFAULT_SYSTEM_PROMPT: &str = "\
Ты анализируешь данные школьного журнала и помогаешь учителю или классному руководителю \
понять ситуацию с учеником. Отвечай по-русски, спокойным деловым тоном.

Что ты получаешь. В каждом запросе приходит текстовая выгрузка по ученику, разбитая на разделы:
УСПЕВАЕМОСТЬ (результаты работ в процентах от максимума), ОЦЕНКИ (5-балльная шкала, где 5 — высшая, \
2 — неудовлетворительно), ПРОПУСКИ (в уроках, с причинами и пометкой, уважительная ли причина), \
ПОРТФОЛИО (достижения, грамоты, благодарности, участие в конкурсах и олимпиадах с уровнем), \
ПОВЕДЕНИЕ (поощрения, замечания, нарушения), ПИТАНИЕ (посещаемость столовой), а также последние работы.

Как пользоваться данными:
1. Опирайся только на то, что есть в выгрузке. Не придумывай оценки, даты, предметы, фамилии, \
причины и события. Если раздел пуст («нет», «не найдено») — скажи, что данных нет, и не делай выводов из пустоты.
2. Приводи конкретные цифры и факты из выгрузки: «средняя оценка по физике 3,2, последние оценки 3, 3, 2», \
«пропущено 9 уроков, из них 6 без уважительной причины». Формулировки вроде «в целом неплохо» без цифр недопустимы.
3. Пропуски по уважительной причине (болезнь, соревнования, семейные обстоятельства) и без неё оценивай по-разному: \
первые — повод проверить, не накопились ли долги по темам; вторые — повод для беседы и работы с семьёй. \
Обращай внимание на закономерности: пропуски одного предмета, один и тот же день недели, всплеск в конкретном месяце.
4. Связывай разделы между собой, если это следует из данных: падение оценок после серии пропусков, рост замечаний при \
снижении успеваемости, сильное портфолио при слабых оценках по профильному предмету. Но не утверждай причинно-следственную \
связь как факт — формулируй как гипотезу, которую стоит проверить.
5. Портфолио учитывай как сильную сторону: уровень (школьный, муниципальный, региональный, всероссийский, международный) \
и вид (грамота, благодарность, диплом, сертификат) показывают вес достижения.
6. Поведение описывай корректно и без ярлыков: пиши о конкретных записях («3 замечания за последний месяц»), \
а не о характере ребёнка. Не ставь диагнозов и не делай психологических выводов.
7. Если питание не отмечено или ученик часто не питается, упоминай это нейтрально и только как факт для внимания классного руководителя.
8. Не копируй строки выгрузки дословно и не перечисляй все записи подряд — выбирай показательные.

Формат ответа: заголовки markdown (##), нумерованные и маркированные списки, абзацы короткие. Рекомендации — конкретные, \
исполнимые учителем (кого позвать на беседу, по какой теме дать дополнительные задания, что обсудить с родителями), \
а не общие слова. В конце коротко укажи, каких данных не хватило для более точных выводов.";

/// Промпт для полного отчёта об ученике.
pub fn build_report_prompt(context: &str, focus: Option<&str>) -> String {
    let mut p = String::from("Составь развёрнутый разбор ситуации с этим учеником.\n\n");
    if let Some(f) = focus {
        if !f.trim().is_empty() {
            p.push_str(&format!("ОСОБЫЙ ФОКУС РАЗБОРА: {}\n\n", f.trim()));
        }
    }
    p.push_str("ДАННЫЕ ОБ УЧЕНИКЕ:\n");
    p.push_str(context);
    p.push_str(
        "\n\nСделай разбор по этим разделам, каждый — отдельным заголовком ##:\n\
1. Краткое резюме — 2–3 предложения с опорой на цифры.\n\
2. Успеваемость и оценки — по предметам, где сильнее и где слабее.\n\
3. Посещаемость и пропуски — объём, причины, закономерности.\n\
4. Портфолио и достижения — что есть и какого уровня.\n\
5. Поведение — только факты из записей.\n\
6. Питание — одной-двумя фразами, если есть данные.\n\
7. Динамика по месяцам — что менялось и как это связано с остальными разделами.\n\
8. Рекомендации учителю и классному руководителю — 3–5 конкретных действий.\n\
9. Чего не хватает в данных.",
    );
    p
}

/// Промпт для сравнения двух учеников.
pub fn build_compare_prompt(a: &str, b: &str) -> String {
    format!(
        "Сравни двух учеников по данным журнала.\n\n\
=== УЧЕНИК A ===\n{a}\n\n=== УЧЕНИК B ===\n{b}\n\n\
Сделай сравнение по пунктам, каждый — заголовком ##:\n\
1. Успеваемость и оценки — кто сильнее и насколько, с конкретными цифрами.\n\
2. Пропуски — объём и причины у каждого.\n\
3. Портфолио — сравни достижения по виду и уровню.\n\
4. Поведение — только факты из записей.\n\
5. Различия в динамике по месяцам.\n\
6. Что нужно ученику A.\n\
7. Что нужно ученику B.\n\
8. Общий вывод для учителя.",
    )
}

/// Промпт для общего обзора класса/потока.
pub fn build_overview_prompt(summary: &str, groups: &str) -> String {
    format!(
        "На основе сводных данных по классу (потоку) напиши аналитическую записку для учителя.\n\n=== ОБЩАЯ СВОДКА ===\n{summary}\n\n=== ПО ГРУППАМ ===\n{groups}\n\n\
Записка должна содержать разделы ##:\n\
1. Общая картина — успеваемость, оценки, посещаемость, цифры.\n\
2. Сравнение групп — какая группа выделяется и почему.\n\
3. Пропуски и поведение — доля пропусков без уважительной причины, замечания против поощрений.\n\
4. Достижения — портфолио.\n\
5. Тревожные сигналы — где есть просадки.\n\
6. Приоритетные действия — 3–5 пунктов.",
    )
}

/// Промпт «расскажи всё про ученика» — свободная форма.
pub fn build_freeform_prompt(context: &str, question: &str) -> String {
    format!(
        "Ниже — данные журнала об ученике. Ответь на вопрос, опираясь только на эти данные. Если ответа в данных нет — так и напиши.\n\n\
ДАННЫЕ ОБ УЧЕНИКЕ:\n{context}\n\nВОПРОС: {question}"
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
        "avg_grade": d.summary.avg_grade,
        "absences": d.summary.absences,
        "absences_unexcused": d.summary.absences_unexcused,
        "portfolio": d.summary.portfolio,
        "praises": d.summary.praises,
        "remarks": d.summary.remarks,
        "meals": d.summary.meals,
        "meals_eaten": d.summary.meals_eaten,
        "total_minutes": d.summary.total_minutes,
        "subjects": d.by_subject,
        "kinds": d.by_kind,
        "timeline": d.timeline,
    })
}
