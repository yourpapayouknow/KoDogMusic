use crate::deps::image;
use crate::domain::error::{AppError, CatalogError};

/// 有界解码外部图片并等比缩放。
pub fn ldimage(bytes: &[u8], side: u32) -> Result<image::RgbaImage, AppError> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()
        .map_err(|error| CatalogError::InvalidData(error.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096); limits.max_image_height = Some(4096); limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|error| CatalogError::InvalidData(error.to_string()))?;
    Ok(image.resize(side.clamp(1, 512), side.clamp(1, 512), image::imageops::FilterType::Triangle).to_rgba8())
}
