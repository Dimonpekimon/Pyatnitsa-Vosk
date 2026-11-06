//! Ошибки мозга.
//!
//! Разделение то же, что в Python-версии: [`BrainError::NextModel`] — «эта
//! модель не подошла, возьми запасную», [`BrainError::Unavailable`] — «дальше
//! пробовать бессмысленно: нет ключа, денег, сети или квоты».

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BrainError {
    /// Мозг недоступен: нет ключа, сети, квоты или модель не найдена.
    #[error("{0}")]
    Unavailable(String),

    /// Внутренний сигнал: переключиться на запасную модель.
    #[error("{0}")]
    NextModel(String),

    /// Локальная ошибка: не прочитался файл, не сжалась картинка.
    #[error("{0}")]
    Io(String),
}

impl BrainError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        BrainError::Unavailable(message.into())
    }

    pub fn next_model(message: impl Into<String>) -> Self {
        BrainError::NextModel(message.into())
    }

    /// Текст без учёта типа — удобно для логов.
    pub fn message(&self) -> &str {
        match self {
            BrainError::Unavailable(text)
            | BrainError::NextModel(text)
            | BrainError::Io(text) => text,
        }
    }
}

impl From<std::io::Error> for BrainError {
    fn from(err: std::io::Error) -> Self {
        BrainError::Io(err.to_string())
    }
}
