//! Провайдеры LLM.
//!
//! Общая часть — разбор ошибок: у шлюзов три разных формата, и все три надо
//! понимать, иначе «нет средств» выглядит как падение, а не как сообщение.

pub mod gemini;
pub mod openai;

use serde_json::Value;

/// Достаёт человекочитаемое сообщение из тела ответа об ошибке.
///
/// OpenAI кладёт `{"error": {"message": ...}}`, aiai.by — `{"error": "..."}`
/// (строку), кто-то отвечает просто текстом.
pub fn error_text(body: &str, status: u16) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let mut message = String::new();

    if let Some(Value::Object(object)) = parsed.as_ref() {
        if let Some(error) = object.get("error") {
            match error {
                Value::Object(details) => {
                    message = details
                        .get("message")
                        .and_then(|v| v.as_str())
                        .or_else(|| details.get("code").and_then(|v| v.as_str()))
                        .unwrap_or_default()
                        .to_string();
                }
                Value::String(text) => message = text.clone(),
                _ => {}
            }
        }
        if message.trim().is_empty() {
            for key in ["message", "detail"] {
                if let Some(text) = object.get(key).and_then(|v| v.as_str()) {
                    message = text.to_string();
                    break;
                }
            }
        }
    } else if let Some(Value::String(text)) = parsed.as_ref() {
        message = text.clone();
    }

    let mut message = message.trim().to_string();
    if message.is_empty() {
        message = body.trim().chars().take(200).collect();
    }
    let message: String = message.chars().take(300).collect();
    if message.trim().is_empty() {
        format!("ошибка {status}")
    } else {
        message
    }
}

/// Google отказывает по географии, а не из-за ключа. Такой ответ приходит с
/// кодом 400 и легко маскируется под «неверный ключ».
pub fn is_region_block(text: &str) -> bool {
    let lowered = text.to_lowercase();
    lowered.contains("location is not supported") || lowered.contains("failed_precondition")
}

/// Обрезает строку по границе символов — чтобы не резать UTF-8 пополам.
pub fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_as_object() {
        let body = r#"{"error": {"code": "INSUFFICIENT_BALANCE", "message": "Недостаточно средств"}}"#;
        assert_eq!(error_text(body, 402), "Недостаточно средств");
    }

    #[test]
    fn error_as_string() {
        let body = r#"{"error": "insufficient balance: have 0.0090 byn"}"#;
        assert_eq!(error_text(body, 402), "insufficient balance: have 0.0090 byn");
    }

    #[test]
    fn error_as_plain_text() {
        assert_eq!(error_text("Bad Gateway", 502), "Bad Gateway");
    }

    #[test]
    fn region_block_detected() {
        assert!(is_region_block("User location is not supported for the API use."));
        assert!(!is_region_block("API key not valid"));
    }
}
