//! Русские подписи для типов записей, статусов и категорий + разбор значений
//! из CSV/JSON (принимаются и русские, и английские варианты).

/// Типы записей (внутренний код → русское название).
pub fn kind_ru(kind: &str) -> &str {
    match kind {
        "task" => "Домашняя работа",
        "quiz" => "Контрольная работа",
        "submission" => "Проект",
        "attendance" => "Занятие",
        "grade" => "Оценка",
        "absence" => "Пропуск",
        "portfolio" => "Портфолио",
        "behavior" => "Поведение",
        "meal" => "Питание",
        other => other,
    }
}

/// Русское или английское название типа → внутренний код.
pub fn kind_from(s: &str) -> String {
    let k = s.trim().to_lowercase();
    match k.as_str() {
        "task" | "homework" | "дз" | "задание" | "домашняя работа" | "домашнее задание" => "task",
        "quiz" | "test" | "тест" | "контрольная" | "контрольная работа" | "самостоятельная" => "quiz",
        "submission" | "project" | "проект" | "проектная работа" => "submission",
        "attendance" | "посещаемость" | "занятие" | "урок" => "attendance",
        "grade" | "оценка" | "отметка" => "grade",
        "absence" | "пропуск" | "пропуски" | "отсутствие" | "н" => "absence",
        "portfolio" | "портфолио" | "достижение" | "достижения" | "грамота" | "благодарность" => "portfolio",
        "behavior" | "behaviour" | "поведение" => "behavior",
        "meal" | "питание" | "еда" | "обед" | "завтрак" | "столовая" => "meal",
        _ => return k,
    }
    .to_string()
}

/// Статусы работ и пропусков (внутренний код → по-русски).
pub fn status_ru(status: &str) -> &str {
    match status {
        "done" => "выполнено вовремя",
        "late" => "с опозданием",
        "missing" => "не выполнено",
        "submitted" => "сдано",
        "excused" => "уважительный",
        "unexcused" => "неуважительный",
        "ate" => "питался",
        "skipped" => "не питался",
        other => other,
    }
}

pub fn status_from(s: &str) -> String {
    let k = s.trim().to_lowercase();
    match k.as_str() {
        "done" | "выполнено" | "выполнено вовремя" | "вовремя" => "done",
        "late" | "с опозданием" | "опоздание" | "выполнено с опозданием" => "late",
        "missing" | "не выполнено" | "не сдано" => "missing",
        "submitted" | "сдано" => "submitted",
        "excused" | "уважительный" | "уважительная" => "excused",
        "unexcused" | "неуважительный" | "неуважительная" => "unexcused",
        "ate" | "питался" | "питалась" | "да" | "ел" | "был" => "ate",
        "skipped" | "не питался" | "не питалась" | "нет" | "не ел" | "не был" | "пропустил" => "skipped",
        _ => return k,
    }
    .to_string()
}

pub fn yes_no(v: bool) -> &'static str {
    if v { "да" } else { "нет" }
}

/// «да/нет/true/1…» → bool.
pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "да" | "yes" | "true" | "1" | "y" | "+" | "уважительная" | "уважительный" => Some(true),
        "нет" | "no" | "false" | "0" | "n" | "-" | "неуважительная" | "неуважительный" => Some(false),
        _ => None,
    }
}

/// Причина пропуска считается уважительной, если явно не сказано обратное.
pub fn reason_is_excused(reason: &str) -> bool {
    let r = reason.trim().to_lowercase();
    if r.is_empty() {
        return false;
    }
    !(r.contains("без уваж") || r.contains("неуваж") || r.contains("прогул") || r.contains("не указан") || r.contains("неизвест"))
}

/// Единый вид категории: первая буква заглавная; синонимы поведения сводятся к 4 значениям.
pub fn norm_category(kind: &str, s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        return String::new();
    }
    let low = t.to_lowercase();
    if kind == "behavior" {
        let c = match low.as_str() {
            "поощрение" | "похвала" | "благодарность" | "praise" | "good" | "+" => "Поощрение",
            "замечание" | "remark" | "warning" => "Замечание",
            "нарушение" | "violation" | "bad" | "-" => "Нарушение",
            "нейтрально" | "нейтральное" | "neutral" => "Нейтрально",
            _ => return capitalize(t),
        };
        return c.to_string();
    }
    capitalize(t)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Заголовок колонки → нормализованный ключ: «Макс. балл» → `макс_балл`.
pub fn norm_header(h: &str) -> String {
    let mut out = String::with_capacity(h.len());
    for ch in h.trim().trim_start_matches('\u{feff}').chars() {
        if ch.is_alphanumeric() || ch == '_' {
            out.extend(ch.to_lowercase());
        } else if ch.is_whitespace() || ch == '-' {
            out.push('_');
        }
        // остальную пунктуацию отбрасываем
    }
    let mut res = String::new();
    let mut prev_us = true;
    for ch in out.chars() {
        if ch == '_' {
            if !prev_us {
                res.push('_');
            }
            prev_us = true;
        } else {
            res.push(ch);
            prev_us = false;
        }
    }
    res.trim_end_matches('_').to_string()
}
