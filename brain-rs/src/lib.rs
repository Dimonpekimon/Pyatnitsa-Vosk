//! Мозг Пятницы на Rust.
//!
//! Порт `brain.py`: тот же протокол, те же сообщения об ошибках, те же
//! настройки — но без Python в горячем пути. Поддерживаются оба провайдера:
//!
//! * `gemini` — родной протокол Google (`generateContent`);
//! * `openai` — любой шлюз на `/v1/chat/completions` (polza.ai, VseGPT,
//!   ProxyAPI, OpenRouter, локальный LiteLLM).
//!
//! Точка входа для ассистента — [`Brain::ask`]. Для интеграции с Python есть
//! бинарь `pyatnitsa-brain` с режимом `stdio` (JSON-строки) и мост `brain_rs.py`.

pub mod brain;
pub mod config;
pub mod error;
pub mod providers;
pub mod text;
pub mod vision;

pub use brain::{AskOptions, Brain};
pub use config::{Provider, Settings, REGION_BLOCK_HINT};
pub use error::BrainError;

/// Версия крейта — печатается в `--version` и уходит в лог при старте.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
