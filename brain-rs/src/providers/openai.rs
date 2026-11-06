//! OpenAI-совместимый шлюз: `/v1/chat/completions`.
//!
//! Так говорят polza.ai, VseGPT, ProxyAPI, OpenRouter, 1app, локальный
//! LiteLLM. Нужен там, где Google блокирует регион.

use serde_json::{json, Value};

/// История + текущий вопрос в формате `chat/completions`.
pub fn build_messages(
    system_prompt: &str,
    history: &[(String, String)],
    prompt: &str,
    image_b64: Option<&str>,
    image_mime: &str,
) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::with_capacity(history.len() * 2 + 2);
    let system_prompt = system_prompt.trim();
    if !system_prompt.is_empty() {
        messages.push(json!({"role": "system", "content": system_prompt}));
    }
    for (question, answer) in history {
        messages.push(json!({"role": "user", "content": question}));
        messages.push(json!({"role": "assistant", "content": answer}));
    }

    if let Some(data) = image_b64 {
        let text = if prompt.is_empty() {
            "Что на этом изображении?"
        } else {
            prompt
        };
        messages.push(json!({
            "role": "user",
            "content": [
                {"type": "text", "text": text},
                {"type": "image_url", "image_url": {"url": format!("data:{image_mime};base64,{data}")}}
            ]
        }));
    } else {
        messages.push(json!({"role": "user", "content": prompt}));
    }
    messages
}

pub fn build_payload(model: &str, messages: &[Value], max_tokens: u32, temperature: f64) -> Value {
    json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
    })
}

/// Текст ответа. `content` бывает строкой или списком частей.
pub fn extract_text(data: &Value) -> String {
    let Some(content) = data
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|choices| choices.first())
        .and_then(|first| first.get("message"))
        .and_then(|message| message.get("content"))
    else {
        return String::new();
    };

    match content {
        Value::String(text) => text.trim().to_string(),
        Value::Array(parts) => {
            let mut chunks = String::new();
            for part in parts {
                if let Some(text) = part
                    .get("text")
                    .and_then(|v| v.as_str())
                    .or_else(|| part.get("content").and_then(|v| v.as_str()))
                {
                    chunks.push_str(text);
                }
            }
            chunks.trim().to_string()
        }
        _ => String::new(),
    }
}

/// Список моделей: `{"data": [{"id": ...}]}` или просто массив.
pub fn models_from_list(data: &Value) -> Vec<String> {
    let items = match data {
        Value::Object(object) => object.get("data").and_then(|v| v.as_array()),
        Value::Array(items) => Some(items),
        _ => None,
    };
    items
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(|v| v.as_str()))
                .filter(|id| !id.is_empty())
                .map(|id| id.to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_carry_system_and_history() {
        let history = vec![("первый вопрос".to_string(), "ответ".to_string())];
        let messages = build_messages("Ты — Пятница.", &history, "второй вопрос", None, "image/png");
        let roles: Vec<&str> = messages
            .iter()
            .map(|m| m["role"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(roles, vec!["system", "user", "assistant", "user"]);
        assert_eq!(messages[0]["content"], "Ты — Пятница.");
        assert_eq!(messages[1]["content"], "первый вопрос");
    }

    #[test]
    fn image_goes_as_data_url() {
        let messages = build_messages("", &[], "что на фото", Some("QUJD"), "image/png");
        let content = &messages[0]["content"];
        assert!(content.is_array());
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,QUJD");
    }

    #[test]
    fn text_from_parts() {
        let data = json!({"choices": [{"message": {"content": [
            {"type": "text", "text": "часть один"},
            {"type": "text", "text": " и два"}
        ]}}]});
        assert_eq!(extract_text(&data), "часть один и два");
    }

    #[test]
    fn text_from_string() {
        let data = json!({"choices": [{"message": {"content": " привет "}}]});
        assert_eq!(extract_text(&data), "привет");
    }

    #[test]
    fn models_from_both_shapes() {
        let wrapped = json!({"data": [{"id": "a"}, {"id": "b"}]});
        assert_eq!(models_from_list(&wrapped), vec!["a", "b"]);
        let bare = json!([{"id": "c"}]);
        assert_eq!(models_from_list(&bare), vec!["c"]);
    }
}
