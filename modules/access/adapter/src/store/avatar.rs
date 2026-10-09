//! Decode only the compact still WebP produced by the profile editor.
use image::{GenericImageView, ImageFormat, ImageReader, Limits};
use nexofolio_access_contracts::{AVATAR_MAX_BYTES, AVATAR_SIZE};
use nexofolio_common::{Error, Result};
use std::io::Cursor;

pub(super) fn validate(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > AVATAR_MAX_BYTES {
        return Err(Error::invalid("头像需小于 256 KiB"));
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::WebP);
    let mut limits = Limits::default();
    limits.max_image_width = Some(AVATAR_SIZE);
    limits.max_image_height = Some(AVATAR_SIZE);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| Error::invalid("头像不是有效的静态 WebP 图片"))?;
    if image.dimensions() != (AVATAR_SIZE, AVATAR_SIZE) {
        return Err(Error::invalid("头像尺寸需为 512 × 512"));
    }
    // Animated and metadata-bearing inputs are unnecessary for a canvas export.
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(Error::invalid("头像容器格式无效"));
    }
    if u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8 != bytes.len() {
        return Err(Error::invalid("头像容器不完整"));
    }
    let mut cursor = 12;
    while cursor < bytes.len() {
        if cursor + 8 > bytes.len() {
            return Err(Error::invalid("头像容器不完整"));
        }
        let kind = &bytes[cursor..cursor + 4];
        if !matches!(kind, b"VP8X" | b"ICCP" | b"ALPH" | b"VP8 " | b"VP8L") {
            return Err(Error::invalid("头像需为无附加元数据的静态图片"));
        }
        let length = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        cursor = cursor
            .checked_add(8 + length + (length & 1))
            .ok_or_else(|| Error::invalid("头像容器不完整"))?;
        if cursor > bytes.len() {
            return Err(Error::invalid("头像容器不完整"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageEncoder, codecs::webp::WebPEncoder};
    fn webp(size: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        WebPEncoder::new_lossless(&mut bytes)
            .write_image(
                &vec![100; (size * size * 3) as usize],
                size,
                size,
                ExtendedColorType::Rgb8,
            )
            .unwrap();
        bytes
    }
    #[test]
    fn validates_decodable_exact_size_images_and_rejects_invalid_inputs() {
        let good = webp(512);
        assert!(validate(&good).is_ok());
        assert!(validate(&webp(256)).is_err());
        assert!(validate(b"not an image").is_err());
        assert!(validate(&good[..good.len() - 4]).is_err());
        assert!(validate(&vec![0; AVATAR_MAX_BYTES + 1]).is_err());
    }
}
