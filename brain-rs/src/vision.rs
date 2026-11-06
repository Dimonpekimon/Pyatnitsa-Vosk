//! Подготовка картинки к отправке.
//!
//! Большой скриншот — это тяжёлый запрос и лишние входные токены, поэтому
//! перед отправкой изображение сжимается до 1600 px по длинной стороне.

use std::path::Path;

use base64::Engine;

use crate::error::BrainError;
use crate::text::mime_for_path;

/// Больше этого размера отправлять накладно.
pub const MAX_INLINE_IMAGE_BYTES: u64 = 3_500_000;
pub const MAX_IMAGE_SIDE: u32 = 1600;

/// Читает файл, при необходимости сжимает и кодирует в base64.
///
/// Возвращает `(base64, mime)`.
pub fn prepare_image(path: &Path) -> Result<(String, String), BrainError> {
    let bytes = std::fs::read(path).map_err(|err| {
        BrainError::Io(format!(
            "Не удалось прочитать изображение {}: {err}",
            path.display()
        ))
    })?;
    let mut mime = mime_for_path(path).to_string();
    let mut data = bytes;

    if data.len() as u64 > MAX_INLINE_IMAGE_BYTES {
        match compress(&data) {
            Ok(compressed) => {
                data = compressed;
                mime = "image/jpeg".to_string();
            }
            Err(err) => {
                // Сжатие — оптимизация, а не условие работы: отправляем как есть.
                eprintln!("Не удалось сжать картинку: {err}");
            }
        }
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
    Ok((encoded, mime))
}

/// Уменьшает изображение и пересохраняет в JPEG q85.
pub fn compress(bytes: &[u8]) -> Result<Vec<u8>, BrainError> {
    let image = image::load_from_memory(bytes)
        .map_err(|err| BrainError::Io(format!("картинка не открылась: {err}")))?;
    let thumbnail = image.thumbnail(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE).to_rgb8();

    let mut buffer: Vec<u8> = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, 85);
    encoder
        .encode_image(&thumbnail)
        .map_err(|err| BrainError::Io(format!("картинка не сжалась: {err}")))?;
    Ok(buffer)
}

/// Кодирует уже готовые байты — используется мостом и тестами.
pub fn encode_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Декодирует base64 (для тестов и отладки).
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn base64_roundtrip() {
        let encoded = encode_base64(b"ABC");
        assert_eq!(encoded, "QUJD");
        assert_eq!(decode_base64(&encoded).unwrap(), b"ABC");
    }

    #[test]
    fn big_image_is_compressed() {
        // Шум: PNG его почти не сжимает, поэтому выигрыш JPEG очевиден.
        let mut img = image::RgbImage::new(2400, 1800);
        let mut seed: u32 = 0x1234_5678;
        for pixel in img.pixels_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *pixel = image::Rgb([seed as u8, (seed >> 8) as u8, (seed >> 16) as u8]);
        }
        let mut raw: Vec<u8> = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut raw), image::ImageFormat::Png)
            .unwrap();
        let compressed = compress(&raw).unwrap();
        assert!(compressed.len() < raw.len());
        assert_eq!(&compressed[..2], &[0xFF, 0xD8], "ожидался JPEG");
    }
}
