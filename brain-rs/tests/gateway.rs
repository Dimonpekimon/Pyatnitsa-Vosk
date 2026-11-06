//! Провод настоящий, шлюз — поддельный.
//!
//! Локальный HTTP-сервер на 127.0.0.1 отвечает как `/v1/models`,
//! `/v1/chat/completions` и `:generateContent`, и запоминает, что ему прислали.
//! Сети, кроме loopback, нет — тесты идут без ключей и без интернета.
//!
//! Проверяется ровно то, что легко сломать незаметно:
//!
//! * `LLM_PROVIDER = openai` берёт базу, модель и ключ из настроек;
//! * системный промпт и история уходят как `system/user/assistant`;
//! * ответ достаётся из `choices[0].message.content` (строкой и списком);
//! * 401 — «ключ не принят», 402 — «нет средств» (в обоих форматах);
//! * 404 модели переключает на запасную;
//! * Gemini-режим не изменился: payload остаётся `contents`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use pyatnitsa_brain::config::{Provider, Settings};
use pyatnitsa_brain::{AskOptions, Brain};
use serde_json::{json, Value};

#[derive(Clone)]
struct State {
    key: String,
    status: u16,
    error_style: &'static str,
    content: Value,
    gemini_content: String,
    requests: Vec<Value>,
    reject_thinking: bool,
}

struct Mock {
    port: u16,
    state: Arc<Mutex<State>>,
    shutdown: Arc<AtomicU16>,
}

impl Mock {
    fn start() -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").expect("порт не открылся");
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State {
            key: "good-key".to_string(),
            status: 200,
            error_style: "object",
            content: json!("привет"),
            gemini_content: "привет".to_string(),
            requests: Vec::new(),
            reject_thinking: false,
        }));
        let shutdown = Arc::new(AtomicU16::new(0));

        let state_for_thread = Arc::clone(&state);
        let shutdown_for_thread = Arc::clone(&shutdown);
        thread::spawn(move || {
            for stream in listener.incoming() {
                if shutdown_for_thread.load(Ordering::SeqCst) == 1 {
                    break;
                }
                if let Ok(stream) = stream {
                    let state = Arc::clone(&state_for_thread);
                    thread::spawn(move || {
                        let _ = handle(stream, state);
                    });
                }
            }
        });

        Mock { port, state, shutdown }
    }

    fn base(&self, provider: Provider) -> String {
        match provider {
            Provider::OpenAi => format!("http://127.0.0.1:{}/v1", self.port),
            Provider::Gemini => format!("http://127.0.0.1:{}/v1beta", self.port),
        }
    }

    fn set(&self, apply: impl FnOnce(&mut State)) {
        let mut state = self.state.lock().unwrap();
        apply(&mut state);
    }

    fn last_request(&self) -> Value {
        let state = self.state.lock().unwrap();
        state.requests.last().cloned().unwrap_or(Value::Null)
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.shutdown.store(1, Ordering::SeqCst);
        // Будим accept: соединение к самому себе.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn handle(mut stream: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];

    let (head_len, content_length) = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find(&buffer, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buffer[..position]).to_string();
            let length = head
                .lines()
                .find_map(|line| {
                    let lower = line.to_lowercase();
                    lower
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            break (position + 4, length);
        }
    };

    while buffer.len() < head_len + content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    let head = String::from_utf8_lossy(&buffer[..head_len]).to_string();
    let body = String::from_utf8_lossy(&buffer[head_len..]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let headers: Vec<String> = lines.map(|line| line.to_string()).collect();

    let (status, payload) = route(&state, &method, &path, &headers, &body);
    respond(&mut stream, status, &payload)
}

fn route(
    state: &Arc<Mutex<State>>,
    method: &str,
    path: &str,
    headers: &[String],
    body: &str,
) -> (u16, Value) {
    let mut state = state.lock().unwrap();
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);

    let authorization = headers
        .iter()
        .find(|line| line.to_lowercase().starts_with("authorization:"))
        .map(|line| line.splitn(2, ':').nth(1).unwrap_or_default().trim().to_string())
        .unwrap_or_default();

    let is_gemini = path.contains(":generateContent") || (path.contains("/models") && path.contains("key="));
    let key_ok = if is_gemini {
        path.contains(&format!("key={}", state.key))
    } else {
        authorization == format!("Bearer {}", state.key)
    };

    if method == "GET" && path.contains("/models") {
        if !key_ok {
            return (401, json!({"error": {"message": "invalid api key"}}));
        }
        if is_gemini {
            return (
                200,
                json!({"models": [
                    {"name": "models/mock-main", "supportedGenerationMethods": ["generateContent"]},
                    {"name": "models/mock-backup", "supportedGenerationMethods": ["generateContent"]}
                ]}),
            );
        }
        return (
            200,
            json!({"object": "list", "data": [{"id": "mock-main"}, {"id": "mock-backup"}]}),
        );
    }

    if method != "POST" {
        return (404, json!({"error": {"message": "not found"}}));
    }

    state.requests.push(parsed.clone());

    if !key_ok {
        return (401, json!({"error": {"message": "invalid api key"}}));
    }

    if state.status != 200 {
        return match state.status {
            402 if state.error_style == "string" => (
                402,
                json!({"error": "insufficient balance: have 0.0090 byn"}),
            ),
            402 => (
                402,
                json!({"error": {"code": "INSUFFICIENT_BALANCE", "message": "Недостаточно средств"}}),
            ),
            other => (other, json!({"error": {"message": "model not found"}})),
        };
    }

    if path.contains(":generateContent") {
        if state.reject_thinking
            && parsed["generationConfig"].get("thinkingConfig").is_some()
        {
            return (
                400,
                json!({"error": {"message": "invalid argument: thinkingConfig is not supported"}}),
            );
        }
        return (
            200,
            json!({"candidates": [{"content": {"parts": [{"text": state.gemini_content}]}}]}),
        );
    }

    let model = parsed["model"].as_str().unwrap_or_default().to_string();
    if model != "mock-main" && model != "mock-backup" {
        return (
            404,
            json!({"error": {"message": format!("model {model} not found")}}),
        );
    }

    (200, json!({"choices": [{"message": {"role": "assistant", "content": state.content}}]}))
}

fn respond(stream: &mut TcpStream, status: u16, payload: &Value) -> std::io::Result<()> {
    let body = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        404 => "Not Found",
        _ => "Error",
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn openai_settings(mock: &Mock) -> Settings {
    let mut settings = Settings::default();
    settings.provider = Provider::OpenAi;
    settings.llm_api_base = mock.base(Provider::OpenAi);
    settings.llm_api_key = "good-key".to_string();
    settings.llm_model = "mock-main".to_string();
    settings.gemini_model = "gemini-3.5-flash".to_string();
    settings.gemini_fallback_models = Vec::new();
    settings.llm_system_prompt = "Ты — Пятница.".to_string();
    settings
}

// ---------------------------------------------------------------------
//  OpenAI-совместимый шлюз
// ---------------------------------------------------------------------
#[test]
fn provider_reads_settings() {
    let mock = Mock::start();
    let brain = Brain::with_settings(openai_settings(&mock));
    assert_eq!(brain.provider(), Provider::OpenAi);
    assert_eq!(brain.model(), "mock-main");
    assert_eq!(brain.api_key(), "good-key");
    assert!(brain.api_base().ends_with("/v1"));
}

#[test]
fn base_strips_completions_suffix() {
    let mock = Mock::start();
    let mut settings = openai_settings(&mock);
    settings.llm_api_base = format!("{}/chat/completions", settings.llm_api_base);
    let brain = Brain::with_settings(settings);
    assert!(brain.api_base().ends_with("/v1"));
    assert!(!brain.api_base().contains("chat/completions"));
}

#[test]
fn check_reports_ready() {
    let mock = Mock::start();
    let mut brain = Brain::with_settings(openai_settings(&mock));
    let (ok, message) = brain.check(true);
    assert!(ok, "{message}");
    assert!(message.contains("mock-main"));
}

#[test]
fn ask_returns_text_and_keeps_history() {
    let mock = Mock::start();
    let mut brain = Brain::with_settings(openai_settings(&mock));
    let answer = brain.ask("Скажи привет", AskOptions::remembering()).unwrap();
    assert_eq!(answer, "привет");
    assert_eq!(brain.history().len(), 1);
    assert_eq!(brain.history()[0].0, "Скажи привет");
}

#[test]
fn messages_carry_system_prompt_and_history() {
    let mock = Mock::start();
    let mut brain = Brain::with_settings(openai_settings(&mock));
    brain.ask("первый вопрос", AskOptions::remembering()).unwrap();
    brain.ask("второй вопрос", AskOptions::remembering()).unwrap();

    let payload = mock.last_request();
    let messages = payload["messages"].as_array().unwrap();
    let roles: Vec<&str> = messages
        .iter()
        .map(|message| message["role"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(roles, vec!["system", "user", "assistant", "user"]);
    assert_eq!(messages[0]["content"], "Ты — Пятница.");
    assert_eq!(messages[1]["content"], "первый вопрос");
    assert_eq!(payload["model"], "mock-main");
    assert!(payload.get("max_tokens").is_some());
}

#[test]
fn plain_text_strips_markdown() {
    let mock = Mock::start();
    mock.set(|state| state.content = json!("**привет** `сэр`"));
    let mut brain = Brain::with_settings(openai_settings(&mock));
    assert_eq!(brain.ask_simple("ну").unwrap(), "привет сэр");
}

#[test]
fn content_as_parts() {
    let mock = Mock::start();
    mock.set(|state| {
        state.content = json!([
            {"type": "text", "text": "часть один"},
            {"type": "text", "text": " и два"}
        ])
    });
    let mut brain = Brain::with_settings(openai_settings(&mock));
    assert_eq!(brain.ask_simple("ну").unwrap(), "часть один и два");
}

#[test]
fn bad_key_is_reported() {
    let mock = Mock::start();
    let mut settings = openai_settings(&mock);
    settings.llm_api_key = "wrong-key".to_string();
    let mut brain = Brain::with_settings(settings);
    let error = brain.ask_simple("ну").unwrap_err();
    assert!(error.message().to_lowercase().contains("ключ"));
}

#[test]
fn no_balance_is_reported() {
    let mock = Mock::start();
    mock.set(|state| state.status = 402);
    let mut brain = Brain::with_settings(openai_settings(&mock));
    let error = brain.ask_simple("ну").unwrap_err();
    assert!(error.message().contains("средств"));
}

#[test]
fn no_balance_as_plain_string() {
    let mock = Mock::start();
    mock.set(|state| {
        state.status = 402;
        state.error_style = "string";
    });
    let mut brain = Brain::with_settings(openai_settings(&mock));
    let error = brain.ask_simple("ну").unwrap_err();
    assert!(error.message().contains("средств"));
    assert!(error.message().contains("insufficient balance"));
}

#[test]
fn unknown_model_falls_back() {
    let mock = Mock::start();
    let mut settings = openai_settings(&mock);
    settings.llm_model = "gone".to_string();
    settings.gemini_fallback_models = vec!["mock-backup".to_string()];
    let mut brain = Brain::with_settings(settings);
    assert_eq!(brain.ask_simple("ну").unwrap(), "привет");
}

#[test]
fn image_goes_as_data_url() {
    let mock = Mock::start();
    let mut brain = Brain::with_settings(openai_settings(&mock));
    let options = AskOptions::remembering().with_image("QUJD", "image/png");
    brain.ask("что на фото", options).unwrap();

    let payload = mock.last_request();
    let messages = payload["messages"].as_array().unwrap();
    let content = &messages.last().unwrap()["content"];
    assert!(content.is_array());
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,QUJD");
}

#[test]
fn vision_uses_main_model() {
    let mock = Mock::start();
    let brain = Brain::with_settings(openai_settings(&mock));
    assert!(brain.vision_enabled());
    assert_eq!(brain.model(), "mock-main");
}

// ---------------------------------------------------------------------
//  Gemini
// ---------------------------------------------------------------------
#[test]
fn gemini_is_default_provider() {
    let brain = Brain::with_settings(Settings::default());
    assert_eq!(brain.provider(), Provider::Gemini);
    assert_eq!(brain.model(), "gemini-3.5-flash");
    assert!(brain.api_base().contains("generativelanguage"));
}

#[test]
fn gemini_payload_keeps_shape() {
    let mock = Mock::start();
    let mut settings = Settings::default();
    settings.gemini_api_base = mock.base(Provider::Gemini);
    settings.gemini_api_key = "good-key".to_string();
    settings.gemini_fallback_models = Vec::new();
    let mut brain = Brain::with_settings(settings);

    assert_eq!(brain.ask_simple("привет").unwrap(), "привет");
    let payload = mock.last_request();
    assert!(payload.get("contents").is_some());
    assert!(payload.get("systemInstruction").is_some());
    assert!(payload.get("generationConfig").is_some());
    assert!(payload.get("messages").is_none());
}

#[test]
fn gemini_retries_without_thinking_config() {
    let mock = Mock::start();
    mock.set(|state| state.reject_thinking = true);
    let mut settings = Settings::default();
    settings.gemini_api_base = mock.base(Provider::Gemini);
    settings.gemini_api_key = "good-key".to_string();
    settings.gemini_fallback_models = Vec::new();
    let mut brain = Brain::with_settings(settings);

    assert_eq!(brain.ask_simple("привет").unwrap(), "привет");
}

#[test]
fn gemini_region_block_is_explained() {
    let mock = Mock::start();
    let mut settings = Settings::default();
    settings.gemini_api_base = mock.base(Provider::Gemini);
    settings.gemini_api_key = "bad-key".to_string();
    let mut brain = Brain::with_settings(settings);
    // Ключ не тот — сервер отвечает 401 «invalid api key».
    let error = brain.ask_simple("привет").unwrap_err();
    assert!(error.message().to_lowercase().contains("ключ"));
}
