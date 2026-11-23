//! Pictures in the chat: attachments and screenshots, decoded once and kept
//! as textures while the chat is open.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use moshionly_ai::ImageData;

/// The longest side a picture is kept at, in pixels. Bigger pictures are
/// scaled down when decoded; the chat never shows them larger.
const MAX_SIDE: u32 = 1024;

/// The largest picture file that can be attached, in bytes.
pub const MAX_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;

/// Decoded pictures, by their content.
#[derive(Default)]
pub struct ImageCache {
    textures: HashMap<u64, Option<TextureHandle>>,
}

impl ImageCache {
    /// The texture for `image`, decoding it the first time. `None` when the
    /// file is not a picture this app can read.
    pub fn texture(&mut self, ctx: &Context, image: &ImageData) -> Option<TextureHandle> {
        let key = content_key(&image.data);
        self.textures
            .entry(key)
            .or_insert_with(|| {
                decode(&image.data).map(|pixels| {
                    ctx.load_texture(
                        format!("chat-image-{key:x}"),
                        pixels,
                        TextureOptions::LINEAR,
                    )
                })
            })
            .clone()
    }

    /// Forgets every texture, for a new conversation.
    pub fn clear(&mut self) {
        self.textures.clear();
    }
}

fn content_key(data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

/// Decodes an image file into pixels, scaled down to [`MAX_SIDE`].
pub fn decode(data: &[u8]) -> Option<ColorImage> {
    let image = image::load_from_memory(data).ok()?;
    let image = if image.width() > MAX_SIDE || image.height() > MAX_SIDE {
        image.thumbnail(MAX_SIDE, MAX_SIDE)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// Why a file could not be attached.
#[derive(Debug, thiserror::Error)]
pub enum AttachError {
    /// The file could not be read.
    #[error("{name} could not be read: {message}")]
    Read {
        /// The file's name.
        name: String,
        /// What the system said.
        message: String,
    },
    /// The file is larger than [`MAX_ATTACHMENT_BYTES`].
    #[error("{name} is larger than 8 MB")]
    TooLarge {
        /// The file's name.
        name: String,
    },
    /// The file is not a PNG, JPEG, GIF or WebP picture.
    #[error("{name} is not a PNG, JPEG, GIF or WebP picture")]
    NotAnImage {
        /// The file's name.
        name: String,
    },
}

/// Reads a picture file for attaching to a message.
pub fn read_attachment(path: &Path) -> Result<ImageData, AttachError> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let data = std::fs::read(path).map_err(|error| AttachError::Read {
        name: name.clone(),
        message: error.to_string(),
    })?;
    if data.len() > MAX_ATTACHMENT_BYTES {
        return Err(AttachError::TooLarge { name });
    }
    let media_type = match image::guess_format(&data) {
        Ok(image::ImageFormat::Png) => "image/png",
        Ok(image::ImageFormat::Jpeg) => "image/jpeg",
        Ok(image::ImageFormat::Gif) => "image/gif",
        Ok(image::ImageFormat::WebP) => "image/webp",
        _ => return Err(AttachError::NotAnImage { name }),
    };
    Ok(ImageData {
        media_type: media_type.to_owned(),
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn decodes_and_scales_down_large_pictures() {
        let small = decode(&png(4, 3)).unwrap();
        assert_eq!(small.size, [4, 3]);
        let large = decode(&png(2048, 512)).unwrap();
        assert_eq!(large.size, [1024, 256]);
        assert!(decode(b"not a picture").is_none());
    }

    #[test]
    fn attachments_are_checked_before_they_are_sent() {
        let dir = std::env::temp_dir().join(format!("moshionly-attach-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let picture = dir.join("shot.png");
        std::fs::write(&picture, png(2, 2)).unwrap();
        let text = dir.join("notes.png");
        std::fs::write(&text, b"hello").unwrap();

        let image = read_attachment(&picture).unwrap();
        assert_eq!(image.media_type, "image/png");
        assert!(matches!(
            read_attachment(&text),
            Err(AttachError::NotAnImage { name }) if name == "notes.png"
        ));
        assert!(matches!(
            read_attachment(&dir.join("missing.png")),
            Err(AttachError::Read { .. })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
}
