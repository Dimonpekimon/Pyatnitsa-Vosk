//! Родной протокол Gemini: `contents` + `systemInstruction` + `generationConfig`.

use serde_json::{json, Value};

use crate::config::Settings;

/// История + текущий вопрос в формате `generateContent`.
pub fn build_contents(
    history: &[(String, String)],
    prompt: &str,
    image_b64: Option<&str>,
    image_mime: &str,
) -> Vec<Value> {
    let mut contents: Vec<Value> = Vec::with_capacity(history.len() * 2 + 1);
    for (question, answer) in history {
        contents.push(json!({"role": "user", "parts": [{"text": question}]}));
        contents.push(json!({"role": "model", "parts": [{"text": answer}]}));
    }

    let mut parts: Vec<Value> = Vec::with_capacity(2);
    if !prompt.is_empty() {
        parts.push(json!({"text": prompt}));
    }
    if let Some(data) = image_b64 {
        parts.push(json!({"inline_data": {"mime_type": image_mime, "data": data}}));
    }
    contents.push(json!({"role": "user", "parts": parts}));
    contents
}

/// Полное тело запроса. `thinking_supported = false` — модель уже отказалась
/// принимать `thinkingConfig`, второй раз не мешаем ей.
pub fn build_payload(
    settings: &Settings,
    contents: Vec<Value>,
    temperature: Option<f64>,
    thinking_supported: bool,
) -> Value {
    let mut payload = json!({ "contents": contents });

    let system_prompt = settings.llm_system_prompt.trim();
    if !system_prompt.is_empty() {
        payload["systemInstruction"] = json!({"parts": [{"text": system_prompt}]});
    }

    let mut generation = json!({
        "temperature": temperature.unwrap_or(settings.gemini_temperature),
        "maxOutputTokens": settings.gemini_max_tokens,
    });
    if let Some(budget) = settings.gemini_thinking_budget {
        if thinking_supported {
            generation["thinkingConfig"] = json!({"thinkingBudget": budget});
        }
    }
    payload["generationConfig"] = generation;
    payload
}

/// Текст ответа. `thought`-части пропускаем: их нельзя озвучивать.
pub fn extract_text(data: &Value) -> String {
    let Some(candidates) = data.get("candidates").and_then(|v| v.as_array()) else {
        return String::new();
    };
    let Some(first) = candidates.first() else {
        return String::new();
    };
    let Some(parts) = first
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(|parts| parts.as_array())
    else {
        return String::new();
    };

    let mut chunks = String::new();
    for part in parts {
        if part.get("thought").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
            chunks.push_str(text);
        }
    }
    chunks.trim().to_string()
}

/// Причина, по которой Gemini ничего не вернула (SAFETY, BLOCKLIST и т.п.).
pub fn block_reason(data: &Value) -> String {
    if let Some(reason) = data
        .get("promptFeedback")
        .and_then(|feedback| feedback.get("blockReason"))
        .and_then(|v| v.as_str())
    {
        if !reason.is_empty() {
            return reason.to_string();
        }
    }
    if let Some(finish) = data
        .get("candidates")
        .and_then(|v| v.as_array())
        .and_then(|candidates| candidates.first())
        .and_then(|first| first.get("finishReason"))
        .and_then(|v| v.as_str())
    {
        if matches!(finish, "SAFETY" | "PROHIBITED_CONTENT" | "BLOCKLIST") {
            return finish.to_string();
        }
    }
    String::new()
}

/// Список моделей из ответа `GET /models` — только те, что умеют генерировать.
pub fn models_from_list(data: &Value) -> Vec<String> {
    let Some(items) = data.get("models").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| {
            item.get("supportedGenerationMethods")
                .and_then(|v| v.as_array())
                .map(|methods| {
                    methods
                        .iter()
                        .any(|method| method.as_str() == Some("generateContent"))
                })
                .unwrap_or(false)
        })
        .filter_map(|item| item.get("name").and_then(|v| v.as_str()))
        .map(|name| name.trim_start_matches("models/").to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_has_gemini_shape() {
        let settings = Settings::default();
        let contents = build_contents(&[], "привет", None, "image/png");
        let payload = build_payload(&settings, contents, None, true);
        assert!(payload.get("contents").is_some());
        assert!(payload.get("systemInstruction").is_some());
        assert!(payload.get("generationConfig").is_some());
        assert!(payload.get("messages").is_none());
        assert_eq!(
            payload["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            json!(0)
        );
    }

    #[test]
    fn history_goes_as_pairs() {
        let history = vec![("вопрос".to_string(), "ответ".to_string())];
        let contents = build_contents(&history, "ещё", None, "image/png");
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[2]["role"], "user");
    }

    #[test]
    fn thinking_config_can_be_dropped() {
        let settings = Settings::default();
        let contents = build_contents(&[], "привет", None, "image/png");
        let payload = build_payload(&settings, contents, None, false);
        assert!(payload["generationConfig"].get("thinkingConfig").is_none());
    }

    #[test]
    fn text_skips_thoughts() {
        let data = json!({"candidates": [{"content": {"parts": [
            {"text": "думаю", "thought": true},
            {"text": "привет"}
        ]}}]});
        assert_eq!(extract_text(&data), "привет");
    }

    #[test]
    fn models_filtered() {
        let data = json!({"models": [
            {"name": "models/a", "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/b", "supportedGenerationMethods": ["embedContent"]}
        ]});
        assert_eq!(models_from_list(&data), vec!["a"]);
    }
}
