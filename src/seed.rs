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
                    if kind == "grade" { Some(grade) } else { None },
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
    }

    Ok(total_activities)
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
