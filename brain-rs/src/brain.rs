//! Мозг: диалог с LLM, переключение моделей, история, зрение.
//!
//! Логика повторяет `brain.py` один в один — включая тексты ошибок: их читает
//! пользователь вслух, поэтому «нет средств» должно остаться «нет средств».

use std::collections::VecDeque;
use std::path::Path;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use reqwest::StatusCode;
use serde_json::Value;

use crate::config::{Provider, Settings, REGION_BLOCK_HINT};
use crate::error::BrainError;
use crate::providers::{error_text, gemini, is_region_block, openai};
use crate::text::plain_text;
use crate::vision;

/// Параметры одного вопроса.
#[derive(Clone, Debug, Default)]
pub struct AskOptions {
    /// Класть ли пару «вопрос-ответ» в историю.
    pub remember: bool,
    /// Готовая картинка в base64.
    pub image_b64: Option<String>,
    pub image_mime: String,
    /// Переопределить модель для этого запроса.
    pub model: Option<String>,
    pub temperature: Option<f64>,
}

impl AskOptions {
    pub fn remembering() -> Self {
        AskOptions {
            remember: true,
            image_mime: "image/png".to_string(),
            ..Default::default()
        }
    }

    pub fn one_shot() -> Self {
        AskOptions {
            remember: false,
            image_mime: "image/png".to_string(),
            ..Default::default()
        }
    }

    pub fn with_image(mut self, data_base64: impl Into<String>, mime: impl Into<String>) -> Self {
        self.image_b64 = Some(data_base64.into());
        self.image_mime = mime.into();
        self
    }
}

/// Мозг Пятницы.
pub struct Brain {
    settings: Settings,
    history: VecDeque<(String, String)>,
    client: Client,
    direct: Option<Client>,
    available: Option<bool>,
    models: Vec<String>,
    probe_after: Option<Instant>,
    last_error: String,
    /// Некоторые модели (flash-lite) не принимают `thinkingConfig`.
    thinking_supported: bool,
    /// Печатать ли отладочные сообщения в stderr.
    pub debug: bool,
}

impl Brain {
    /// Собирает мозг из настроек по умолчанию (файл + окружение).
    pub fn new() -> Self {
        Brain::with_settings(Settings::load(None))
    }

    pub fn with_settings(settings: Settings) -> Self {
        let client = Client::builder()
            .user_agent(format!("pyatnitsa-brain/{}", crate::VERSION))
            .build()
            .expect("HTTP-клиент не собрался");
        Brain {
            settings,
            history: VecDeque::new(),
            client,
            direct: None,
            available: None,
            models: Vec::new(),
            probe_after: None,
            last_error: String::new(),
            thinking_supported: true,
            debug: std::env::var_os("PYATNITSA_BRAIN_DEBUG").is_some(),
        }
    }

    // ------------------------------------------------------------------
    //  Свойства
    // ------------------------------------------------------------------
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn provider(&self) -> Provider {
        self.settings.provider
    }

    pub fn model(&self) -> String {
        self.settings.default_model()
    }

    pub fn api_base(&self) -> String {
        self.settings.api_base()
    }

    pub fn api_key(&self) -> String {
        self.settings.api_key()
    }

    pub fn key_present(&self) -> bool {
        !self.api_key().is_empty()
    }

    pub fn history(&self) -> &VecDeque<(String, String)> {
        &self.history
    }

    pub fn last_error(&self) -> &str {
        &self.last_error
    }

    pub fn reset(&mut self) {
        self.history.clear();
    }

    pub fn vision_enabled(&self) -> bool {
        self.settings.vision_enabled()
    }

    pub fn vision_available(&self) -> bool {
        self.vision_enabled() && self.key_present()
    }

    fn log(&self, message: &str, debug_only: bool) {
        if debug_only && !self.debug {
            return;
        }
        eprintln!("{message}");
    }

    // ------------------------------------------------------------------
    //  Ключ и доступность
    // ------------------------------------------------------------------
    fn require_key(&self) -> Result<String, BrainError> {
        let key = self.api_key();
        if key.is_empty() {
            return Err(BrainError::unavailable(
                "Ключ LLM не задан. Впишите GEMINI_API_KEY (или LLM_API_KEY) \
                 в pyatnitsa_settings.json либо задайте переменную окружения.",
            ));
        }
        Ok(key)
    }

    /// Список моделей, доступных этому ключу. Кэшируется на `GEMINI_RECHECK_SEC`.
    pub fn available_models(&mut self, force: bool) -> Vec<String> {
        if !self.key_present() {
            self.available = Some(false);
            self.models.clear();
            self.last_error = "Ключ LLM не задан. Впишите GEMINI_API_KEY (или LLM_API_KEY) \
                 в pyatnitsa_settings.json."
                .to_string();
            return Vec::new();
        }
        if !force && self.available.is_some() && self.probe_after.is_some_and(|at| Instant::now() < at)
        {
            return self.models.clone();
        }

        match self.settings.provider {
            Provider::OpenAi => self.probe_openai_models(),
            Provider::Gemini => self.probe_gemini_models(),
        }
    }

    fn probe_gemini_models(&mut self) -> Vec<String> {
        let base = self.api_base();
        let url = format!("{base}/models");
        let key = self.api_key();

        let response = match self.send(|client| {
            client
                .get(&url)
                .query(&[("key", key.as_str())])
                .timeout(Duration::from_secs(10))
                .send()
        }) {
            Ok(response) => response,
            Err(err) => {
                self.fail(format!(
                    "Нет связи с Gemini ({}). Проверьте интернет и прокси.",
                    network_reason(&err)
                ));
                return Vec::new();
            }
        };

        let status = response.status();
        let body = response.text().unwrap_or_default();

        if matches!(status.as_u16(), 400 | 401 | 403) {
            let message = if is_region_block(&body) {
                REGION_BLOCK_HINT.to_string()
            } else {
                "Ключ Gemini неверный или отозван. Получите новый в Google AI Studio \
                 и обновите GEMINI_API_KEY."
                    .to_string()
            };
            self.fail(message);
            return Vec::new();
        }
        if status.is_client_error() || status.is_server_error() {
            self.fail(format!("Gemini вернула ошибку {}.", status.as_u16()));
            return Vec::new();
        }

        let Ok(data) = serde_json::from_str::<Value>(&body) else {
            self.fail("Gemini вернула некорректный ответ.".to_string());
            return Vec::new();
        };

        self.models = gemini::models_from_list(&data);
        self.available = Some(true);
        self.last_error.clear();
        self.schedule_recheck();
        self.models.clone()
    }

    fn probe_openai_models(&mut self) -> Vec<String> {
        let base = self.api_base();
        let url = format!("{base}/models");
        let key = self.api_key();

        let response = match self.send(|client| {
            client
                .get(&url)
                .bearer_auth(&key)
                .timeout(Duration::from_secs(10))
                .send()
        }) {
            Ok(response) => response,
            Err(err) => {
                self.fail(format!(
                    "Нет связи с {base} ({}). Проверьте адрес шлюза, интернет и прокси.",
                    network_reason(&err)
                ));
                return Vec::new();
            }
        };

        let status = response.status();
        let body = response.text().unwrap_or_default();

        if matches!(status.as_u16(), 401 | 403) {
            self.fail(format!(
                "Шлюз {base} не принял ключ ({}). Проверьте LLM_API_KEY.",
                error_text(&body, status.as_u16())
            ));
            return Vec::new();
        }
        if status.is_client_error() || status.is_server_error() {
            self.fail(format!(
                "Шлюз {base} ответил ошибкой {}: {}",
                status.as_u16(),
                error_text(&body, status.as_u16())
            ));
            return Vec::new();
        }

        let Ok(data) = serde_json::from_str::<Value>(&body) else {
            self.fail("Шлюз вернул некорректный список моделей.".to_string());
            return Vec::new();
        };

        self.models = openai::models_from_list(&data);
        self.available = Some(true);
        self.last_error.clear();
        self.schedule_recheck();
        self.models.clone()
    }

    fn fail(&mut self, message: String) {
        self.available = Some(false);
        self.models.clear();
        self.last_error = message;
        self.schedule_recheck();
    }

    fn schedule_recheck(&mut self) {
        let recheck = self.settings.gemini_recheck_secs.max(0.0);
        self.probe_after = Some(Instant::now() + Duration::from_secs_f64(recheck));
    }

    pub fn is_available(&mut self) -> bool {
        if self.available.is_none() {
            self.available_models(false);
        }
        self.available.unwrap_or(false)
    }

    pub fn model_present(&mut self) -> bool {
        if self.models.is_empty() {
            self.available_models(false);
        }
        let model = self.model();
        self.models.iter().any(|item| item == &model)
    }

    /// `(ок, сообщение)` — для стартовой диагностики.
    pub fn check(&mut self, force: bool) -> (bool, String) {
        if !self.key_present() {
            return (
                false,
                "Ключ LLM не задан. Впишите GEMINI_API_KEY (или LLM_API_KEY) \
                 в pyatnitsa_settings.json либо задайте переменную окружения."
                    .to_string(),
            );
        }
        let models = self.available_models(force);
        if models.is_empty() {
            let message = if self.last_error.is_empty() {
                "LLM не отвечает.".to_string()
            } else {
                self.last_error.clone()
            };
            return (false, message);
        }

        let model = self.model();
        if !models.iter().any(|item| item == &model) {
            let preview = models
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            let preview = if preview.is_empty() { "нет".to_string() } else { preview };
            return (
                false,
                format!("Модель «{model}» недоступна для этого ключа. Доступны, например: {preview}."),
            );
        }

        if self.settings.provider == Provider::OpenAi {
            (
                true,
                format!("Шлюз {} готов, модель «{model}».", self.api_base()),
            )
        } else {
            (true, format!("Gemini готова, модель «{model}»."))
        }
    }

    /// Проверяет ключ и модель. Генерацию не запускает — не тратит квоту.
    pub fn warmup(&mut self) -> bool {
        let (ok, message) = self.check(false);
        if ok {
            self.log(&format!("Мозг готов: {message}"), true);
        } else {
            self.log(&message, false);
        }
        ok
    }

    // ------------------------------------------------------------------
    //  Диалог
    // ------------------------------------------------------------------
    pub fn ask(&mut self, prompt: &str, options: AskOptions) -> Result<String, BrainError> {
        self.require_key()?;
        if prompt.is_empty() && options.image_b64.is_none() {
            return Err(BrainError::unavailable(
                "Пустой запрос: нечего отправлять в LLM.",
            ));
        }

        let target = options.model.clone().unwrap_or_else(|| self.model());
        let mut candidates = vec![target.clone()];
        for fallback in self.settings.gemini_fallback_models.clone() {
            if !fallback.trim().is_empty() && !candidates.contains(&fallback) {
                candidates.push(fallback);
            }
        }

        let mut last_error = String::new();
        let mut data: Option<Value> = None;
        let mut used_model = target;

        for candidate in &candidates {
            let attempt = match self.settings.provider {
                Provider::OpenAi => self.post_openai(candidate, prompt, &options),
                Provider::Gemini => self.post_gemini(candidate, prompt, &options),
            };
            match attempt {
                Ok(value) => {
                    data = Some(value);
                    used_model = candidate.clone();
                    break;
                }
                Err(BrainError::NextModel(message)) => {
                    last_error = message;
                    self.log(&format!("{last_error} Пробую запасную модель."), true);
                }
                Err(other) => return Err(other),
            }
        }

        let Some(data) = data else {
            return Err(BrainError::unavailable(if last_error.is_empty() {
                "Ни одна модель не ответила. Попробуйте позже.".to_string()
            } else {
                last_error
            }));
        };

        let raw = match self.settings.provider {
            Provider::OpenAi => openai::extract_text(&data),
            Provider::Gemini => gemini::extract_text(&data),
        };
        let answer = plain_text(&raw);

        if answer.is_empty() {
            let reason = if self.settings.provider == Provider::Gemini {
                gemini::block_reason(&data)
            } else {
                String::new()
            };
            self.log(&format!("Пустой ответ LLM ({used_model})."), false);
            return Ok(if reason.is_empty() {
                "Сэр, модель вернула пустой ответ.".to_string()
            } else {
                format!("Сэр, Gemini отклонила запрос: {reason}.")
            });
        }

        if options.remember {
            self.push_history(prompt.to_string(), answer.clone());
        }
        Ok(answer)
    }

    /// Короткая форма: спросить и запомнить.
    pub fn ask_simple(&mut self, prompt: &str) -> Result<String, BrainError> {
        self.ask(prompt, AskOptions::remembering())
    }

    /// Вопрос по картинке. В историю не попадает — это разовый анализ.
    pub fn ask_with_image(&mut self, prompt: &str, image_path: &Path) -> Result<String, BrainError> {
        if !self.vision_enabled() {
            return Err(BrainError::unavailable(
                "Модель для анализа изображений не задана. \
                 Укажите GEMINI_VISION_MODEL, например «gemini-3.5-flash».",
            ));
        }
        let (data, mime) = vision::prepare_image(image_path)?;
        let model = self.settings.vision_model();
        let options = AskOptions {
            remember: false,
            image_b64: Some(data),
            image_mime: mime,
            model: Some(model),
            temperature: None,
        };
        self.ask(prompt, options)
    }

    fn push_history(&mut self, question: String, answer: String) {
        let limit = self.settings.history_size.max(1);
        while self.history.len() >= limit {
            self.history.pop_front();
        }
        self.history.push_back((question, answer));
    }

    // ------------------------------------------------------------------
    //  Транспорт
    // ------------------------------------------------------------------
    fn direct_client(&mut self) -> &Client {
        if self.direct.is_none() {
            let client = Client::builder()
                .user_agent(format!("pyatnitsa-brain/{}", crate::VERSION))
                .no_proxy()
                .build()
                .expect("прямой HTTP-клиент не собрался");
            self.direct = Some(client);
        }
        self.direct.as_ref().expect("клиент только что создан")
    }

    /// Запрос с запасным путём: если прокси не ответил — идём напрямую.
    fn send<F>(&mut self, build: F) -> Result<Response, reqwest::Error>
    where
        F: Fn(&Client) -> Result<Response, reqwest::Error>,
    {
        match build(&self.client) {
            Ok(response) => Ok(response),
            Err(err) if is_proxy_error(&err) => {
                self.log("Прокси не ответил — пробую напрямую.", true);
                build(self.direct_client())
            }
            Err(err) => Err(err),
        }
    }

    fn post_gemini(
        &mut self,
        model: &str,
        prompt: &str,
        options: &AskOptions,
    ) -> Result<Value, BrainError> {
        let key = self.require_key()?;
        let base = self.api_base();
        let url = format!("{base}/models/{model}:generateContent");
        let history: Vec<(String, String)> = self.history.iter().cloned().collect();
        let contents = gemini::build_contents(
            &history,
            prompt,
            options.image_b64.as_deref(),
            &options.image_mime,
        );
        let payload = gemini::build_payload(
            &self.settings,
            contents,
            options.temperature,
            self.thinking_supported,
        );

        let (status, body) = self.gemini_send(&url, &key, &payload, model)?;
        let code = status.as_u16();

        if code == 429 {
            std::thread::sleep(Duration::from_millis(1500));
            let (retry_status, retry_body) = self.gemini_send(&url, &key, &payload, model)?;
            if retry_status.as_u16() == 429 {
                return Err(BrainError::unavailable(
                    "Бесплатный лимит Gemini исчерпан. Подождите минуту и повторите.",
                ));
            }
            return self.gemini_interpret(model, retry_status, retry_body, &payload);
        }

        self.gemini_interpret(model, status, body, &payload)
    }

    fn gemini_send(
        &mut self,
        url: &str,
        key: &str,
        payload: &Value,
        model: &str,
    ) -> Result<(StatusCode, String), BrainError> {
        let timeout = self.settings.gemini_timeout_secs.max(1.0);
        let response = self.send(|client| {
            client
                .post(url)
                .query(&[("key", key)])
                .json(payload)
                .timeout(Duration::from_secs_f64(timeout))
                .send()
        });

        match response {
            Ok(response) => {
                let status = response.status();
                let body = response.text().unwrap_or_default();
                Ok((status, body))
            }
            Err(err) if err.is_timeout() => Err(BrainError::next_model(format!(
                "Модель «{model}» не ответила за {} с.",
                format_secs(timeout)
            ))),
            Err(err) => Err(BrainError::unavailable(format!(
                "Нет связи с Gemini ({}). Проверьте интернет.",
                network_reason(&err)
            ))),
        }
    }

    fn gemini_interpret(
        &mut self,
        model: &str,
        status: StatusCode,
        body: String,
        payload: &Value,
    ) -> Result<Value, BrainError> {
        let code = status.as_u16();
        if code == 404 {
            return Err(BrainError::next_model(format!(
                "Модель «{model}» недоступна для этого ключа."
            )));
        }
        if matches!(code, 400 | 401 | 403) {
            let detail = error_text(&body, code);
            // Часть моделей (flash-lite) не принимает thinkingConfig и отвечает
            // просто «invalid argument». Повторяем без параметра.
            if code == 400 && payload["generationConfig"].get("thinkingConfig").is_some() {
                self.thinking_supported = false;
                self.log(
                    &format!("«{model}» не приняла thinkingConfig — повторяю без него."),
                    true,
                );
                let mut retry = payload.clone();
                if let Some(generation) = retry["generationConfig"].as_object_mut() {
                    generation.remove("thinkingConfig");
                }
                let key = self.require_key()?;
                let url = format!("{}/models/{model}:generateContent", self.api_base());
                let (retry_status, retry_body) = self.gemini_send(&url, &key, &retry, model)?;
                return self.gemini_interpret(model, retry_status, retry_body, &retry);
            }
            if is_region_block(&detail) {
                return Err(BrainError::unavailable(REGION_BLOCK_HINT));
            }
            if matches!(code, 401 | 403) || detail.to_lowercase().contains("api key") {
                return Err(BrainError::unavailable(
                    "Ключ Gemini неверный или отозван. Обновите GEMINI_API_KEY.",
                ));
            }
            return Err(BrainError::unavailable(format!(
                "Gemini отклонила запрос: {detail}"
            )));
        }
        if status.is_server_error() {
            return Err(BrainError::next_model(format!(
                "Сервис Gemini временно недоступен ({code})."
            )));
        }

        serde_json::from_str::<Value>(&body)
            .map_err(|_| BrainError::unavailable("Gemini вернула некорректный ответ."))
    }

    fn post_openai(
        &mut self,
        model: &str,
        prompt: &str,
        options: &AskOptions,
    ) -> Result<Value, BrainError> {
        let key = self.require_key()?;
        let base = self.api_base();
        let url = format!("{base}/chat/completions");
        let history: Vec<(String, String)> = self.history.iter().cloned().collect();
        let messages = openai::build_messages(
            &self.settings.llm_system_prompt,
            &history,
            prompt,
            options.image_b64.as_deref(),
            &options.image_mime,
        );
        let payload = openai::build_payload(
            model,
            &messages,
            self.settings.gemini_max_tokens,
            options.temperature.unwrap_or(self.settings.gemini_temperature),
        );
        let timeout = self.settings.gemini_timeout_secs.max(1.0);

        let send = |brain: &mut Brain| -> Result<(StatusCode, String), BrainError> {
            let response = brain.send(|client| {
                client
                    .post(&url)
                    .bearer_auth(&key)
                    .json(&payload)
                    .timeout(Duration::from_secs_f64(timeout))
                    .send()
            });
            match response {
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().unwrap_or_default();
                    Ok((status, body))
                }
                Err(err) if err.is_timeout() => Err(BrainError::next_model(format!(
                    "Модель «{model}» не ответила за {} с.",
                    format_secs(timeout)
                ))),
                Err(err) => Err(BrainError::unavailable(format!(
                    "Нет связи с {base} ({}). Проверьте интернет, адрес шлюза и прокси.",
                    network_reason(&err)
                ))),
            }
        };

        let (mut status, mut body) = send(self)?;
        if status.as_u16() == 429 {
            std::thread::sleep(Duration::from_millis(1500));
            let (retry_status, retry_body) = send(self)?;
            if retry_status.as_u16() == 429 {
                return Err(BrainError::unavailable(
                    "Лимит шлюза исчерпан. Подождите минуту и повторите.",
                ));
            }
            status = retry_status;
            body = retry_body;
        }

        let code = status.as_u16();
        if matches!(code, 401 | 403) {
            return Err(BrainError::unavailable(format!(
                "Шлюз не принял ключ ({}). Проверьте LLM_API_KEY.",
                error_text(&body, code)
            )));
        }
        if code == 404 {
            return Err(BrainError::next_model(format!(
                "Модель «{model}» недоступна на этом шлюзе."
            )));
        }
        if code == 400 {
            let detail = error_text(&body, code);
            let lowered = detail.to_lowercase();
            if lowered.contains("model") && lowered.contains(&model.to_lowercase()) {
                return Err(BrainError::next_model(format!(
                    "Модель «{model}» не найдена: {detail}"
                )));
            }
            return Err(BrainError::unavailable(format!(
                "Шлюз отклонил запрос: {detail}"
            )));
        }
        if status.is_server_error() {
            return Err(BrainError::next_model(format!(
                "Шлюз временно недоступен ({code})."
            )));
        }
        if status.is_client_error() {
            let detail = error_text(&body, code);
            let lowered = detail.to_lowercase();
            if code == 402 || lowered.contains("balance") || lowered.contains("средств") {
                return Err(BrainError::unavailable(format!(
                    "На счету шлюза нет средств: {detail}"
                )));
            }
            return Err(BrainError::unavailable(format!(
                "Шлюз отклонил запрос ({code}): {detail}"
            )));
        }

        serde_json::from_str::<Value>(&body)
            .map_err(|_| BrainError::unavailable("Шлюз вернул некорректный ответ."))
    }
}

impl Default for Brain {
    fn default() -> Self {
        Brain::new()
    }
}

/// Прокси-ошибка reqwest не имеет отдельного типа — ищем её в цепочке причин.
fn is_proxy_error(err: &reqwest::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(current) = source {
        if current.to_string().to_lowercase().contains("proxy") {
            return true;
        }
        source = current.source();
    }
    false
}

fn network_reason(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "таймаут"
    } else if err.is_connect() {
        "не удалось подключиться"
    } else {
        "сетевая ошибка"
    }
}

/// `30` вместо `30.0` — чтобы текст ошибки читался по-русски.
fn format_secs(value: f64) -> String {
    if (value - value.round()).abs() < f64::EPSILON {
        format!("{}", value.round() as i64)
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_without_network() -> Settings {
        let mut settings = Settings::default();
        settings.gemini_api_key = "test-key".to_string();
        settings.llm_system_prompt = String::new();
        settings
    }

    #[test]
    fn empty_prompt_is_rejected() {
        let mut brain = Brain::with_settings(settings_without_network());
        let err = brain.ask("", AskOptions::remembering()).unwrap_err();
        assert!(err.message().contains("Пустой запрос"));
    }

    #[test]
    fn missing_key_is_reported() {
        let mut settings = Settings::default();
        settings.gemini_api_key = String::new();
        settings.llm_api_key = String::new();
        let mut brain = Brain::with_settings(settings);
        let err = brain.ask("привет", AskOptions::remembering()).unwrap_err();
        assert!(err.message().contains("Ключ LLM не задан"));
        assert!(!brain.key_present());
    }

    #[test]
    fn history_is_capped() {
        let mut brain = Brain::with_settings(settings_without_network());
        for index in 0..20 {
            brain.push_history(format!("q{index}"), format!("a{index}"));
        }
        assert_eq!(brain.history().len(), brain.settings().history_size);
        assert_eq!(brain.history().back().unwrap().0, "q19");
    }

    #[test]
    fn secs_are_formatted_without_trailing_zero() {
        assert_eq!(format_secs(30.0), "30");
        assert_eq!(format_secs(2.5), "2.5");
    }
}
