//! Uploaded images are decoded and re-encoded, never stored as sent. That drops EXIF (GPS
//! included) and scales them down to at most 2048 px on the long side.

use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, codecs::jpeg::JpegEncoder, imageops::FilterType};

const MAX_SIDE: u32 = 2048;

pub struct Processed {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    pub content_type: &'static str,
}

pub fn process(input: &[u8]) -> Result<Processed, String> {
    let reader = ImageReader::new(Cursor::new(input)).with_guessed_format().map_err(|e| e.to_string())?;
    match reader.format() {
        Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP) => {}
        _ => return Err("Only JPEG, PNG and WebP images can be uploaded.".into()),
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    // Phones store rotation in EXIF; apply it before EXIF is dropped.
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| format!("Couldn't read the image: {e}"))?;
    img.apply_orientation(orientation);

    if img.width() > MAX_SIDE || img.height() > MAX_SIDE {
        img = img.resize(MAX_SIDE, MAX_SIDE, FilterType::Lanczos3);
    }

    let mut bytes = Vec::new();
    if img.color().has_alpha() {
        img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png).map_err(|e| e.to_string())?;
        Ok(Processed { bytes, ext: "png", content_type: "image/png" })
    } else {
        JpegEncoder::new_with_quality(&mut bytes, 85).encode_image(&img.to_rgb8()).map_err(|e| e.to_string())?;
        Ok(Processed { bytes, ext: "jpg", content_type: "image/jpeg" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = RgbImage::from_pixel(w, h, Rgb([200, 100, 50]));
        let mut out = Vec::new();
        JpegEncoder::new(&mut out).encode_image(&img).unwrap();
        out
    }

    /// Insert an APP1 EXIF segment right after the JPEG SOI marker.
    fn with_exif(jpeg: &[u8]) -> Vec<u8> {
        let payload = b"Exif\0\0MM\0*\0\0\0\x08\0\0GPS-SECRET-59.9139N";
        let len = (payload.len() + 2) as u16;
        let mut out = jpeg[..2].to_vec();
        out.extend([0xFF, 0xE1]);
        out.extend(len.to_be_bytes());
        out.extend(payload);
        out.extend(&jpeg[2..]);
        out
    }

    #[test]
    fn strips_metadata_and_scales_down() {
        let input = with_exif(&jpeg(3000, 1000));
        assert!(input.windows(10).any(|w| w == b"GPS-SECRET"));
        let out = process(&input).unwrap();
        assert_eq!(out.content_type, "image/jpeg");
        assert!(!out.bytes.windows(10).any(|w| w == b"GPS-SECRET"));
        let decoded = image::load_from_memory(&out.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2048, 683));
    }

    #[test]
    fn rejects_other_formats() {
        assert!(process(b"GIF89a....").is_err());
        assert!(process(b"not an image").is_err());
    }
}
