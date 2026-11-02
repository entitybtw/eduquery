//! Демонстрационные данные: ученики и их учебная активность.
//!
//! Генерируются детерминированно (фиксированный seed), чтобы интерфейс
//! сразу выглядел живым. Кнопка «Сгенерировать демо-данные» в UI
//! пересоздаёт этот набор.

use anyhow::Result;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rusqlite::params;

use crate::db::Db;

const GROUPS: &[&str] = &["ИП-21", "ИП-22", "БИТ-21", "БИТ-22", "ПО-23"];
const SUBJECTS: &[&str] = &[
    "Математика",
    "Физика",
    "Программирование",
    "Базы данных",
    "Английский язык",
    "Дискретная математика",
];
const FIRST_NAMES: &[&str] = &[
    "Александр", "Дмитрий", "Екатерина", "Фёдор", "Алиса", "Иван", "Мария", "Николай", "Ольга",
    "Сергей", "Анна", "Пётр", "Ксения", "Артём", "Вера", "Павел", "Юлия", "Роман",
];
const LAST_NAMES: &[&str] = &[
    "Иванов", "Петров", "Сидоров", "Кузнецова", "Смирнова", "Волков", "Морозова", "Зайцев",
    "Павлова", "Семёнов", "Голубева", "Виноградов", "Богданова", "Воробьёв", "Фёдорова",
    "Михайлов", "Беляева", "Тарасов",
];
const TOPICS: &[&str] = &[
    "Домашняя работа №{}",
    "Лабораторная работа №{}",
    "Практическое занятие №{}",
    "Контрольная работа №{}",
    "Проектное задание №{}",
];

/// Заполнить БД демонстрационными учениками и активностью.
pub fn seed(db: &Db) -> Result<usize> {
    let mut rng = StdRng::seed_from_u64(20260101);
    let conn = db.conn();

    conn.execute("DELETE FROM analyses", [])?;
    conn.execute("DELETE FROM activities", [])?;
    conn.execute("DELETE FROM students", [])?;

    let mut total_activities = 0usize;

    for i in 0..24 {
        let first = FIRST_NAMES[rng.gen_range(0..FIRST_NAMES.len())];
        let last = LAST_NAMES[rng.gen_range(0..LAST_NAMES.len())];
        let full_name = format!("{first} {last}");
        let external_id = format!("STU-{:04}", 1001 + i);
        let group = GROUPS[rng.gen_range(0..GROUPS.len())];
        let grade = rng.gen_range(1..=4);
        let email = format!(
            "{}.{}@edu.local",
            translit(first).to_lowercase(),
            translit(last).to_lowercase()
        );

        conn.execute(
            "INSERT INTO students(external_id, full_name, group_name, email, grade)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![external_id, full_name, group, email, grade],
        )?;
        let student_id = conn.last_insert_rowid();

        let n_tasks = rng.gen_range(8..26);
        for t in 0..n_tasks {
            let subject = SUBJECTS[rng.gen_range(0..SUBJECTS.len())];
            let kind = match rng.gen_range(0..10) {
                0..=4 => "task",
                5..=6 => "quiz",
                7 => "submission",
                8 => "attendance",
                _ => "grade",
            };
            let title = match kind {
                "quiz" => format!("Контрольная по «{subject}»"),
                "attendance" => format!("Занятие: {subject}"),
                "grade" => format!("Итоговая оценка: {subject}"),
                _ => TOPICS[rng.gen_range(0..TOPICS.len())].replace("{}", &format!("{}", t + 1)),
            };

            let max_score = 100.0;
            // Сильнее ученики получают больше баллов; добавляем шум.
            let base = 0.35 + grade as f64 * 0.14;
            let ratio = (base + rng.gen_range(-0.22..0.18)).clamp(0.05, 1.0);
            let score = (max_score * ratio * 10.0).round() / 10.0;

            let status = if score / max_score > 0.85 {
                "done"
            } else if score / max_score > 0.5 {
                "late"
            } else {
                "missing"
            };

            let duration = match kind {
                "attendance" => Some(rng.gen_range(45.0..95.0)),
                "task" | "submission" => Some(rng.gen_range(20.0..240.0)),
                "quiz" => Some(rng.gen_range(15.0..60.0)),
                _ => None,
            };

            // Активность распределена по последним ~120 дням.
            let days_ago = rng.gen_range(0..120);
            let hour = rng.gen_range(8..21);
            // Даты в ISO-формате, чтобы SQLite корректно их сортировал.
            // Разброс — за последние ~120 дней от «сегодня».
            let day = chrono::Utc::now().date_naive() - chrono::Duration::days(days_ago as i64);
            let occurred_at = format!(
                "{} {:02}:{:02}:00",
                day.format("%Y-%m-%d"),
                hour,
                rng.gen_range(0..60)
            );

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
                    if kind == "grade" {
                        Some(match ratio {
                            r if r > 0.85 => 5,
                            r if r > 0.65 => 4,
                            r if r > 0.45 => 3,
                            _ => 2,
                        })
                    } else {
                        None
                    },
                    status,
                    duration,
                    occurred_at,
                    serde_json::json!({
                        "note": note_for(kind, &mut rng),
                    })
                    .to_string(),
                ],
            )?;
            total_activities += 1;
        }

        // Сила ученика 0..1 — от неё зависят пропуски, портфолио и поведение.
        let strength = (grade as f64 - 1.0) / 3.0 + rng.gen_range(-0.2..0.2);
        total_activities += seed_extras(&conn, &mut rng, student_id, strength.clamp(0.0, 1.0))?;
    }

    Ok(total_activities)
}

const ABSENCE_REASONS_OK: &[&str] = &[
    "Болезнь", "Болезнь", "Справка от врача", "Семейные обстоятельства",
    "Участие в соревнованиях", "Участие в олимпиаде",
];
const ABSENCE_REASONS_BAD: &[&str] = &["Без уважительной причины", "Прогул", "Опоздание более 20 минут"];
const PORTFOLIO: &[(&str, &str, &str)] = &[
    ("Грамота", "Школьный", "Победитель школьной олимпиады по математике"),
    ("Грамота", "Муниципальный", "Призёр городской олимпиады по физике"),
    ("Диплом", "Региональный", "Диплом II степени, региональный конкурс проектов"),
    ("Сертификат", "Всероссийский", "Участник Всероссийской олимпиады школьников"),
    ("Благодарность", "Школьный", "Помощь в организации школьного праздника"),
    ("Благодарность", "Школьный", "Активное участие в жизни класса"),
    ("Достижение", "Муниципальный", "1 место в турнире по шахматам"),
    ("Достижение", "Региональный", "Призёр соревнований по лёгкой атлетике"),
    ("Грамота", "Международный", "Участник международного конкурса по программированию"),
];
const PRAISES: &[&str] = &[
    "Помог одноклассникам с проектом", "Отличная работа на уроке", "Дежурил по классу без напоминаний",
    "Выступил на школьной линейке",
];
const REMARKS: &[&str] = &[
    "Разговаривал на уроке", "Не выполнил домашнее задание", "Забыл учебные принадлежности",
    "Опоздал на урок", "Пользовался телефоном на уроке",
];
const VIOLATIONS: &[&str] = &["Конфликт с одноклассником на перемене", "Порча школьного имущества"];

/// Пропуски, портфолио, поведение и питание одного ученика.
fn seed_extras(
    conn: &rusqlite::Connection,
    rng: &mut StdRng,
    student_id: i64,
    strength: f64,
) -> Result<usize> {
    let mut n = 0usize;
    let today = chrono::Utc::now().date_naive();
    let date_at = |rng: &mut StdRng| -> String {
        let d = today - chrono::Duration::days(rng.gen_range(0..120) as i64);
        format!("{} 00:00:00", d.format("%Y-%m-%d"))
    };
    let put = |kind: &str, title: &str, subject: &str, status: &str, at: &str,
                   reason: &str, category: &str, level: &str, comment: &str, amount: f64| -> Result<()> {
        conn.execute(
            "INSERT INTO activities(student_id, kind, title, subject, status, occurred_at, details,
                                    reason, category, level, comment, amount)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, '{}', ?7, ?8, ?9, ?10, ?11)",
            params![student_id, kind, title, subject, status, at, reason, category, level, comment, amount],
        )?;
        Ok(())
    };

    // Пропуски: у более слабых учеников их больше и чаще без уважительной причины.
    let n_abs = ((1.0 - strength) * 9.0).round() as i32 + rng.gen_range(0..4);
    for _ in 0..n_abs {
        let bad = rng.gen_bool(((1.0 - strength) * 0.55).clamp(0.05, 0.6));
        let reason = if bad {
            ABSENCE_REASONS_BAD[rng.gen_range(0..ABSENCE_REASONS_BAD.len())]
        } else {
            ABSENCE_REASONS_OK[rng.gen_range(0..ABSENCE_REASONS_OK.len())]
        };
        let subject = if rng.gen_bool(0.6) { SUBJECTS[rng.gen_range(0..SUBJECTS.len())] } else { "" };
        put("absence", "Пропуск занятия", subject, if bad { "unexcused" } else { "excused" },
            &date_at(rng), reason, "", "", "", rng.gen_range(1..=3) as f64)?;
        n += 1;
    }

    // Портфолио: чем сильнее ученик, тем больше достижений.
    let n_port = (strength * 4.0).round() as i32 + rng.gen_range(0..2) - if rng.gen_bool(0.35) { 1 } else { 0 };
    for _ in 0..n_port.max(0) {
        let (cat, level, title) = PORTFOLIO[rng.gen_range(0..PORTFOLIO.len())];
        put("portfolio", title, "", "done", &date_at(rng), "", cat, level, "", 1.0)?;
        n += 1;
    }

    // Поведение: поощрения у сильных, замечания у слабых.
    for _ in 0..(strength * 3.0).round() as i32 + rng.gen_range(0..2) {
        let c = PRAISES[rng.gen_range(0..PRAISES.len())];
        put("behavior", "Поощрение", "", "done", &date_at(rng), "", "Поощрение", "", c, 1.0)?;
        n += 1;
    }
    for _ in 0..((1.0 - strength) * 4.0).round() as i32 + rng.gen_range(0..2) {
        let (cat, c) = if rng.gen_bool(0.12) {
            ("Нарушение", VIOLATIONS[rng.gen_range(0..VIOLATIONS.len())])
        } else {
            ("Замечание", REMARKS[rng.gen_range(0..REMARKS.len())])
        };
        put("behavior", cat, "", "done", &date_at(rng), "", cat, "", c, 1.0)?;
        n += 1;
    }

    // Питание: ~30 последних учебных дней, обед (+ завтрак у части учеников).
    let payment = ["Бесплатное", "Льготное", "Платное"][rng.gen_range(0..3)];
    let breakfast = rng.gen_bool(0.4);
    let eat_prob = if payment == "Платное" { 0.7 } else { 0.9 };
    for d in 0..40 {
        let day = today - chrono::Duration::days(d);
        if matches!(day.format("%a").to_string().as_str(), "Sat" | "Sun") {
            continue;
        }
        let at = format!("{} 00:00:00", day.format("%Y-%m-%d"));
        for meal in if breakfast { vec!["Завтрак", "Обед"] } else { vec!["Обед"] } {
            let ate = rng.gen_bool(eat_prob);
            put("meal", meal, "", if ate { "ate" } else { "skipped" }, &at, "", meal, payment, "", 1.0)?;
            n += 1;
        }
    }
    Ok(n)
}

fn note_for(kind: &str, rng: &mut StdRng) -> String {
    let notes = match kind {
        "task" => vec!["Решено верно", "Есть ошибки в вычислениях", "Сдано с опозданием"],
        "quiz" => vec!["Хороший результат", "Плохо усвоена тема", "Пересдача"],
        "submission" => vec!["Проект принят", "Проект на доработке"],
        "attendance" => vec!["Присутствовал", "Опоздал", "Отсутствовал"],
        _ => vec!["Хорошая оценка", "Нужна доработка"],
    };
    notes[rng.gen_range(0..notes.len())].to_string()
}

/// Грубая транслитерация кириллицы — для «почтовых» идентификаторов.
fn translit(s: &str) -> String {
    const MAP: &[(char, &str)] = &[
        ('а', "a"), ('б', "b"), ('в', "v"), ('г', "g"), ('д', "d"), ('е', "e"), ('ё', "e"),
        ('ж', "zh"), ('з', "z"), ('и', "i"), ('й', "y"), ('к', "k"), ('л', "l"), ('м', "m"),
        ('н', "n"), ('о', "o"), ('п', "p"), ('р', "r"), ('с', "s"), ('т', "t"), ('у', "u"),
        ('ф', "f"), ('х', "h"), ('ц', "c"), ('ч', "ch"), ('ш', "sh"), ('щ', "sch"), ('ъ', ""),
        ('ы', "y"), ('ь', ""), ('э', "e"), ('ю', "yu"), ('я', "ya"),
    ];
    let mut out = String::new();
    for ch in s.chars() {
        let lower = ch.to_lowercase().next().unwrap_or(ch);
        if let Some((_, rep)) = MAP.iter().find(|(c, _)| *c == lower) {
            out.push_str(rep);
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if ch == ' ' {
            out.push('-');
        }
    }
    out
}
