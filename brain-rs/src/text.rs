//! Подготовка текста к озвучке.
//!
//! Синтезатор читает звёздочки и решётки вслух, поэтому markdown вычищается
//! до того, как ответ попадёт в TTS. Набор правил повторяет `brain.plain_text`.

use std::sync::OnceLock;

use regex::Regex;

struct Patterns {
    bold: Regex,
    italic: Regex,
    underline: Regex,
    // Одиночное подчёркивание — тоже курсив, но в brain.py оно не вычищалось.
    // Скобки-границы (вместо lookaround, которого нет в `regex`) не дают
    // испортить snake_case: «foo_bar_baz» остаётся как есть.
    single_underline: Regex,
    code: Regex,
    heading: Regex,
    bullet: Regex,
    numbered: Regex,
    link: Regex,
    spaces: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| Patterns {
        bold: Regex::new(r"\*\*(.+?)\*\*").expect("regex bold"),
        italic: Regex::new(r"\*(.+?)\*").expect("regex italic"),
        underline: Regex::new(r"__(.+?)__").expect("regex underline"),
        single_underline: Regex::new(r"(^|[^\w])_([^_\n]+)_($|[^\w])")
            .expect("regex single underline"),
        code: Regex::new(r"`{1,3}([^`]+)`{1,3}").expect("regex code"),
        heading: Regex::new(r"(?m)^#{1,6}\s*").expect("regex heading"),
        bullet: Regex::new(r"(?m)^\s*[-*+]\s+").expect("regex bullet"),
        numbered: Regex::new(r"(?m)^\s*\d+\.\s+").expect("regex numbered"),
        link: Regex::new(r"\[(.+?)\]\(.+?\)").expect("regex link"),
        spaces: Regex::new(r"\s{2,}").expect("regex spaces"),
    })
}

/// Убирает markdown, чтобы синтезатор не читал разметку.
pub fn plain_text(text: &str) -> String {
    let p = patterns();
    let mut result = text.to_string();
    // Двойное подчёркивание — раньше одиночного, иначе «__x__» съестся как «_x_».
    for regex in [&p.bold, &p.underline] {
        result = regex.replace_all(&result, "$1").into_owned();
    }
    result = p
        .single_underline
        .replace_all(&result, "$1$2$3")
        .into_owned();
    for regex in [
        &p.italic,
        &p.code,
        &p.heading,
        &p.bullet,
        &p.numbered,
        &p.link,
    ] {
        result = regex.replace_all(&result, "$1").into_owned();
    }
    let result = result.replace('—', " — ");
    p.spaces.replace_all(&result, " ").trim().to_string()
}

/// MIME по расширению — без внешних таблиц.
pub fn mime_for_path(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "png" => "image/png",
        _ => "image/png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_markdown() {
        assert_eq!(plain_text("**привет** `сэр`"), "привет сэр");
        // Переводы строк остаются: так же ведёт себя brain.py.
        assert_eq!(plain_text("# Заголовок\n- пункт\n1. шаг"), "Заголовок\nпункт\nшаг");
        assert_eq!(plain_text("см. [ссылку](https://x.y)"), "см. ссылку");
        assert_eq!(plain_text("_курсив_ и __жирный__"), "курсив и жирный");
    }

    #[test]
    fn keeps_snake_case_identifiers() {
        // Одиночное подчёркивание внутри слова — не разметка.
        assert_eq!(plain_text("вызови foo_bar_baz()"), "вызови foo_bar_baz()");
        assert_eq!(plain_text("_a_ и _b_"), "a и b");
    }

    #[test]
    fn collapses_spaces_and_pads_dash() {
        assert_eq!(plain_text("раз — два"), "раз — два");
        assert_eq!(plain_text("a   b"), "a b");
    }

    #[test]
    fn mime_by_extension() {
        use std::path::Path;
        assert_eq!(mime_for_path(Path::new("a.JPG")), "image/jpeg");
        assert_eq!(mime_for_path(Path::new("a.png")), "image/png");
        assert_eq!(mime_for_path(Path::new("a")), "image/png");
    }
}
