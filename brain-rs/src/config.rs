//! Настройки мозга.
//!
//! Источники, в порядке возрастания приоритета:
//! 1. значения по умолчанию (как в `config.py`);
//! 2. `pyatnitsa_settings.json` (тот же файл, что у Python-версии);
//! 3. переменные окружения.
//!
//! Ключ в репозиторий не попадает: он живёт только в файле настроек или в
//! окружении.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Подсказка на случай, когда Google блокирует регион, а не ключ.
pub const REGION_BLOCK_HINT: &str = "Gemini недоступен из этой сети: Google блокирует регион \
(«User location is not supported»). Ключ при этом рабочий — включите VPN или прокси и повторите.";

pub const DEFAULT_GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

pub const DEFAULT_SYSTEM_PROMPT: &str = "Ты — Пятница, голосовой ассистент уровня ИИ из фильмов \
про Железного человека. Ты работаешь на компьютере пользователя под Windows. Отвечай КРАТКО — \
1-3 предложения, если не попросили подробнее. Пиши обычным текстом без markdown, списков и \
эмодзи — ответ будет озвучен вслух. Обращайся к пользователю «сэр». Не выдумывай факты: если не \
знаешь — честно скажи.";

/// Какой протокол использовать.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// Родной протокол Google: `generateContent`.
    Gemini,
    /// Любой шлюз, совместимый с `/v1/chat/completions`.
    OpenAi,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini",
            Provider::OpenAi => "openai",
        }
    }

    /// Неизвестное значение не ломает запуск: как и в Python-версии, берём Gemini.
    pub fn parse(value: &str) -> Provider {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai" => Provider::OpenAi,
            _ => Provider::Gemini,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub provider: Provider,
    pub gemini_api_key: String,
    pub gemini_api_base: String,
    pub gemini_model: String,
    pub gemini_fallback_models: Vec<String>,
    pub gemini_vision_model: String,
    pub gemini_timeout_secs: f64,
    pub gemini_max_tokens: u32,
    pub gemini_temperature: f64,
    pub gemini_thinking_budget: Option<i32>,
    pub gemini_recheck_secs: f64,
    pub history_size: usize,
    pub llm_api_base: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub llm_system_prompt: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            provider: Provider::Gemini,
            gemini_api_key: String::new(),
            gemini_api_base: DEFAULT_GEMINI_API_BASE.to_string(),
            gemini_model: "gemini-3.5-flash".to_string(),
            gemini_fallback_models: vec!["gemini-3.5-flash-lite".to_string()],
            gemini_vision_model: "gemini-3.5-flash".to_string(),
            gemini_timeout_secs: 30.0,
            gemini_max_tokens: 500,
            gemini_temperature: 0.6,
            gemini_thinking_budget: Some(0),
            gemini_recheck_secs: 30.0,
            history_size: 8,
            llm_api_base: String::new(),
            llm_api_key: String::new(),
            llm_model: String::new(),
            llm_system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        }
    }
}

impl Settings {
    /// Ищет файл настроек: явный путь → `PYATNITSA_SETTINGS` → текущий каталог →
    /// каталог бинаря → его родитель (крейт лежит в `brain-rs/`, файл — выше).
    pub fn find_settings_file(explicit: Option<&Path>) -> Option<PathBuf> {
        if let Some(path) = explicit {
            return Some(path.to_path_buf());
        }
        if let Some(path) = std::env::var_os("PYATNITSA_SETTINGS") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("pyatnitsa_settings.json"));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("pyatnitsa_settings.json"));
                if let Some(up) = dir.parent() {
                    candidates.push(up.join("pyatnitsa_settings.json"));
                    if let Some(up2) = up.parent() {
                        candidates.push(up2.join("pyatnitsa_settings.json"));
                    }
                }
            }
        }
        candidates.into_iter().find(|path| path.is_file())
    }

    /// Загружает настройки: файл (если найден) + переменные окружения.
    pub fn load(explicit: Option<&Path>) -> Settings {
        let mut settings = Settings::default();
        if let Some(path) = Settings::find_settings_file(explicit) {
            match std::fs::read_to_string(&path) {
                Ok(raw) => match serde_json::from_str::<Value>(&raw) {
                    Ok(value) => settings.apply_json(&value),
                    Err(err) => eprintln!(
                        "Настройки {} не разобраны ({}), беру значения по умолчанию.",
                        path.display(),
                        err
                    ),
                },
                Err(err) => eprintln!("Не удалось прочитать {}: {}", path.display(), err),
            }
        }
        settings.apply_env();
        settings
    }

    fn apply_json(&mut self, value: &Value) {
        let Some(object) = value.as_object() else {
            return;
        };

        if let Some(provider) = get_str(object, "LLM_PROVIDER") {
            self.provider = Provider::parse(&provider);
        }
        if let Some(key) = get_str(object, "GEMINI_API_KEY") {
            self.gemini_api_key = key;
        }
        if let Some(base) = get_str(object, "GEMINI_API_BASE") {
            if !base.is_empty() {
                self.gemini_api_base = base;
            }
        }
        if let Some(model) = get_str(object, "GEMINI_MODEL") {
            if !model.is_empty() {
                self.gemini_model = model;
            }
        }
        if let Some(models) = get_str_list(object, "GEMINI_FALLBACK_MODELS") {
            self.gemini_fallback_models = models;
        }
        if let Some(model) = get_str(object, "GEMINI_VISION_MODEL") {
            self.gemini_vision_model = model;
        }
        if let Some(value) = get_f64(object, "GEMINI_TIMEOUT") {
            if value > 0.0 {
                self.gemini_timeout_secs = value;
            }
        }
        if let Some(value) = get_u32(object, "GEMINI_MAX_TOKENS") {
            if value > 0 {
                self.gemini_max_tokens = value;
            }
        }
        if let Some(value) = get_f64(object, "GEMINI_TEMPERATURE") {
            self.gemini_temperature = value;
        }
        if object.contains_key("GEMINI_THINKING_BUDGET") {
            self.gemini_thinking_budget = get_i32(object, "GEMINI_THINKING_BUDGET");
        }
        if let Some(value) = get_f64(object, "GEMINI_RECHECK_SEC") {
            if value >= 0.0 {
                self.gemini_recheck_secs = value;
            }
        }
        if let Some(value) = get_u32(object, "LLM_HISTORY_SIZE") {
            self.history_size = value as usize;
        }
        if let Some(base) = get_str(object, "LLM_API_BASE") {
            self.llm_api_base = base;
        }
        if let Some(key) = get_str(object, "LLM_API_KEY") {
            self.llm_api_key = key;
        }
        if let Some(model) = get_str(object, "LLM_MODEL") {
            self.llm_model = model;
        }
        if let Some(prompt) = get_str(object, "LLM_SYSTEM_PROMPT") {
            self.llm_system_prompt = prompt;
        }
    }

    fn apply_env(&mut self) {
        if let Some(value) = env_str("LLM_PROVIDER") {
            self.provider = Provider::parse(&value);
        }
        if let Some(value) = env_str("GEMINI_API_KEY") {
            self.gemini_api_key = value;
        }
        if let Some(value) = env_str("GEMINI_API_BASE") {
            if !value.is_empty() {
                self.gemini_api_base = value;
            }
        }
        if let Some(value) = env_str("GEMINI_MODEL") {
            if !value.is_empty() {
                self.gemini_model = value;
            }
        }
        if let Some(value) = env_str("GEMINI_FALLBACK_MODELS") {
            self.gemini_fallback_models = split_list(&value);
        }
        if let Some(value) = env_str("GEMINI_VISION_MODEL") {
            self.gemini_vision_model = value;
        }
        if let Some(value) = env_f64("GEMINI_TIMEOUT") {
            if value > 0.0 {
                self.gemini_timeout_secs = value;
            }
        }
        if let Some(value) = env_f64("GEMINI_MAX_TOKENS") {
            if value > 0.0 {
                self.gemini_max_tokens = value as u32;
            }
        }
        if let Some(value) = env_f64("GEMINI_TEMPERATURE") {
            self.gemini_temperature = value;
        }
        if std::env::var_os("GEMINI_THINKING_BUDGET").is_some() {
            self.gemini_thinking_budget = env_f64("GEMINI_THINKING_BUDGET").map(|v| v as i32);
        }
        if let Some(value) = env_f64("GEMINI_RECHECK_SEC") {
            if value >= 0.0 {
                self.gemini_recheck_secs = value;
            }
        }
        if let Some(value) = env_f64("LLM_HISTORY_SIZE") {
            self.history_size = value.max(0.0) as usize;
        }
        if let Some(value) = env_str("LLM_API_BASE") {
            self.llm_api_base = value;
        }
        if let Some(value) = env_str("LLM_API_KEY") {
            self.llm_api_key = value;
        }
        if let Some(value) = env_str("LLM_MODEL") {
            self.llm_model = value;
        }
        if let Some(value) = env_str("LLM_SYSTEM_PROMPT") {
            self.llm_system_prompt = value;
        }
    }

    /// Ключ: для шлюза сначала `LLM_API_KEY`, иначе общий `GEMINI_API_KEY`.
    pub fn api_key(&self) -> String {
        if self.provider == Provider::OpenAi {
            let key = self.llm_api_key.trim();
            if !key.is_empty() {
                return key.to_string();
            }
        }
        self.gemini_api_key.trim().to_string()
    }

    /// Модель по умолчанию: `LLM_MODEL` для шлюза, `GEMINI_MODEL` для Google.
    pub fn default_model(&self) -> String {
        if self.provider == Provider::OpenAi {
            let model = self.llm_model.trim();
            if !model.is_empty() {
                return model.to_string();
            }
            return self.gemini_model.clone();
        }
        self.gemini_model.clone()
    }

    /// Адрес API без хвостового слэша и без лишнего `/chat/completions`.
    pub fn api_base(&self) -> String {
        let raw = if self.provider == Provider::OpenAi {
            &self.llm_api_base
        } else {
            &self.gemini_api_base
        };
        resolve_base(raw)
    }

    /// Есть ли у провайдера «зрительная» модель.
    pub fn vision_enabled(&self) -> bool {
        if self.provider == Provider::OpenAi {
            !self.default_model().is_empty()
        } else {
            !self.gemini_vision_model.trim().is_empty()
        }
    }

    /// Модель для анализа картинок: у шлюза отдельной обычно нет.
    pub fn vision_model(&self) -> String {
        if self.provider == Provider::OpenAi {
            self.default_model()
        } else {
            self.gemini_vision_model.trim().to_string()
        }
    }
}

/// Убирает хвосты, которые легко скопировать из документации шлюза.
pub fn resolve_base(raw: &str) -> String {
    let mut base = raw.trim().trim_end_matches('/').to_string();
    for tail in ["/chat/completions", "/completions"] {
        if base.ends_with(tail) {
            base.truncate(base.len() - tail.len());
        }
    }
    base.trim_end_matches('/').to_string()
}

fn get_str(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(|value| value.as_str()).map(|s| s.to_string())
}

fn get_str_list(object: &serde_json::Map<String, Value>, key: &str) -> Option<Vec<String>> {
    match object.get(key)? {
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect(),
        ),
        // Строка вместо списка — частая опечатка в настройках.
        Value::String(text) => Some(split_list(text)),
        _ => None,
    }
}

fn get_f64(object: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    object.get(key).and_then(|value| value.as_f64())
}

fn get_u32(object: &serde_json::Map<String, Value>, key: &str) -> Option<u32> {
    get_f64(object, key).filter(|v| *v >= 0.0).map(|v| v as u32)
}

fn get_i32(object: &serde_json::Map<String, Value>, key: &str) -> Option<i32> {
    match object.get(key)? {
        Value::Null => None,
        value => value.as_f64().map(|v| v as i32),
    }
}

fn env_str(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn env_f64(key: &str) -> Option<f64> {
    std::env::var(key).ok()?.trim().parse::<f64>().ok()
}

fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_is_stripped() {
        assert_eq!(resolve_base("https://api.polza.ai/v1/"), "https://api.polza.ai/v1");
        assert_eq!(
            resolve_base("https://api.polza.ai/v1/chat/completions"),
            "https://api.polza.ai/v1"
        );
        assert_eq!(resolve_base("https://x/v1/completions/"), "https://x/v1");
    }

    #[test]
    fn unknown_provider_falls_back_to_gemini() {
        assert_eq!(Provider::parse("ollama"), Provider::Gemini);
        assert_eq!(Provider::parse("OpenAI"), Provider::OpenAi);
    }

    #[test]
    fn openai_key_falls_back_to_gemini_key() {
        let mut settings = Settings::default();
        settings.provider = Provider::OpenAi;
        settings.llm_api_key = String::new();
        settings.gemini_api_key = "g-key".into();
        assert_eq!(settings.api_key(), "g-key");

        settings.llm_api_key = "llm-key".into();
        assert_eq!(settings.api_key(), "llm-key");
    }

    #[test]
    fn fallback_models_accept_string() {
        let value = serde_json::json!({"GEMINI_FALLBACK_MODELS": "a, b"});
        let mut settings = Settings::default();
        settings.apply_json(&value);
        assert_eq!(settings.gemini_fallback_models, vec!["a", "b"]);
    }
}
