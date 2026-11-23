//! Images: the pictures of image layers, inside the Lottie file or beside
//! it.
//!
//! A PNG or JPEG is written as it is. Any other kind of picture (WebP, GIF,
//! BMP), and a photo whose orientation tag turns it, is decoded, turned
//! upright as the renderer turns it, and written as PNG. Each picture is
//! written once, however many layers show it.
//!
//! An image layer shows its picture at the picture's own size; its
//! transform stretches it to fill the layer's box the way the layer's fit
//! says, and a mask cuts it to the box's rounded corners.

use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, metadata::Orientation};
use moshionly_core::eval::Property;
use moshionly_core::io::{self as core_io, Asset};
use moshionly_core::model::{ImageFit, ImageLayer};
use moshionly_core::{Node, Rect};
use serde_json::{Map, Value};
use tempfile::TempPath;

use crate::error::ExportError;

use super::LottieImages;
use super::groups::{Masks, Place};
use super::layers::{Comp, Converter, Parts};
use super::timeline::{places_for, tolerance};

/// A picture, ready to be written.
#[derive(Debug, Clone, PartialEq)]
struct Picture {
    /// The encoded bytes.
    bytes: Vec<u8>,
    /// PNG or JPEG.
    format: ImageFormat,
    /// Width and height in pixels, upright.
    size: (u32, u32),
}

/// Reads a picture from its encoded bytes, keeping PNG and JPEG as they
/// are and turning everything else into an upright PNG.
fn picture(bytes: Vec<u8>) -> Result<Picture, String> {
    let (format, size, decoded) = {
        let reader = ImageReader::new(Cursor::new(bytes.as_slice()))
            .with_guessed_format()
            .map_err(|error| error.to_string())?;
        let format = reader.format();
        let mut decoder = reader.into_decoder().map_err(|error| error.to_string())?;
        let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        let size = decoder.dimensions();
        let kept = matches!(format, Some(ImageFormat::Png | ImageFormat::Jpeg));
        let decoded = if kept && orientation == Orientation::NoTransforms {
            None
        } else {
            let mut decoded =
                DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
            decoded.apply_orientation(orientation);
            Some(decoded)
        };
        (format, size, decoded)
    };
    let Some(decoded) = decoded else {
        return Ok(Picture {
            bytes,
            format: format.unwrap_or(ImageFormat::Png),
            size,
        });
    };
    let mut png = Cursor::new(Vec::new());
    decoded
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(Picture {
        bytes: png.into_inner(),
        format: ImageFormat::Png,
        size: (decoded.width(), decoded.height()),
    })
}

/// Where a picture went: its asset id and size.
#[derive(Debug, Clone, PartialEq)]
struct Written {
    id: String,
    size: (u32, u32),
}

/// The pictures of an export, and where they are written.
pub(crate) struct Images {
    mode: LottieImages,
    project_file: Option<PathBuf>,
    /// The folder pictures are written into, beside the Lottie file, and
    /// its name as the file refers to it.
    folder: PathBuf,
    folder_name: String,
    /// Each source looked at, and what became of it.
    seen: HashMap<String, Result<Written, String>>,
    /// The files written so far.
    files: ImageFiles,
}

impl Images {
    /// Pictures for a Lottie file going to `destination`, found next to
    /// `project_file`.
    pub fn new(mode: LottieImages, destination: &Path, project_file: Option<&Path>) -> Self {
        let stem = destination.file_stem().map_or_else(
            || "animation".to_owned(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        let folder_name = format!("{stem}_images");
        let folder = destination
            .parent()
            .unwrap_or(Path::new("."))
            .join(&folder_name);
        Self {
            mode,
            project_file: project_file.map(Path::to_owned),
            folder,
            folder_name,
            seen: HashMap::new(),
            files: ImageFiles::default(),
        }
    }

    /// The files written beside the Lottie file, to move into place once
    /// it is written.
    pub fn into_files(self) -> Option<ImageFiles> {
        (!self.files.pending.is_empty()).then_some(self.files)
    }

    /// The asset for `source`, adding it to `assets` the first time.
    fn asset(&mut self, source: &str, assets: &mut Vec<Value>) -> Result<Written, String> {
        if let Some(seen) = self.seen.get(source) {
            return seen.clone();
        }
        let written = self.write(source, assets);
        self.seen.insert(source.to_owned(), written.clone());
        written
    }

    fn write(&mut self, source: &str, assets: &mut Vec<Value>) -> Result<Written, String> {
        let bytes = match core_io::resolve_asset(source, self.project_file.as_deref())
            .map_err(|error| error.to_string())?
        {
            Asset::Data { bytes, .. } => bytes,
            Asset::File(path) => fs::read(&path)
                .map_err(|error| format!("{} could not be read: {error}", path.display()))?,
        };
        let picture = picture(bytes)?;
        let written = self.seen.values().filter(|seen| seen.is_ok()).count();
        let id = format!("image_{written}");
        let (media, extension) = match picture.format {
            ImageFormat::Jpeg => ("image/jpeg", "jpg"),
            _ => ("image/png", "png"),
        };
        let (width, height) = picture.size;
        let mut asset = Map::new();
        asset.insert("id".into(), Value::from(id.as_str()));
        asset.insert("w".into(), Value::from(width));
        asset.insert("h".into(), Value::from(height));
        match self.mode {
            LottieImages::Embedded => {
                asset.insert("u".into(), Value::from(""));
                asset.insert(
                    "p".into(),
                    Value::from(core_io::data_uri(media, &picture.bytes)),
                );
                asset.insert("e".into(), Value::from(1));
            }
            LottieImages::Files => {
                let name = format!("{id}.{extension}");
                self.files
                    .add(&self.folder, &name, &picture.bytes)
                    .map_err(|error| format!("it could not be written beside the file: {error}"))?;
                asset.insert("u".into(), Value::from(format!("{}/", self.folder_name)));
                asset.insert("p".into(), Value::from(name));
                asset.insert("e".into(), Value::from(0));
            }
        }
        assets.push(Value::Object(asset));
        Ok(Written {
            id,
            size: picture.size,
        })
    }
}

/// Image files written beside a Lottie file. They are written as hidden
/// temporary files and renamed into place only after the Lottie file
/// itself; an export that fails or is cancelled leaves none of them, and
/// removes the folder when it made it.
#[derive(Debug, Default)]
pub(crate) struct ImageFiles {
    /// Temporary files and the names they get.
    pending: Vec<(TempPath, PathBuf)>,
    /// The folder, when the export made it.
    made: Option<PathBuf>,
}

impl ImageFiles {
    fn add(&mut self, folder: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        if !folder.is_dir() {
            fs::create_dir(folder)?;
            self.made = Some(folder.to_owned());
        }
        let mut file = tempfile::Builder::new()
            .prefix(&format!(".{name}."))
            .suffix(".part")
            .tempfile_in(folder)?;
        file.write_all(bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(file.path(), fs::Permissions::from_mode(0o644))?;
        }
        self.pending
            .push((file.into_temp_path(), folder.join(name)));
        Ok(())
    }

    /// Moves the image files into place.
    pub(crate) fn commit(mut self) -> Result<(), ExportError> {
        for (temporary, destination) in std::mem::take(&mut self.pending) {
            temporary
                .persist(&destination)
                .map_err(|error| ExportError::io("moved into place", &destination, error.error))?;
        }
        self.made = None;
        Ok(())
    }
}

impl Drop for ImageFiles {
    fn drop(&mut self) {
        // Temporary files delete themselves; a folder made for them goes
        // too when nothing else is in it.
        self.pending.clear();
        if let Some(folder) = self.made.take() {
            let _ = fs::remove_dir(folder);
        }
    }
}

/// Where a picture of `image` size goes in a box of `size`, by `fit`, as
/// the renderer places it.
fn fitted(fit: ImageFit, size: (f64, f64), image: (f64, f64)) -> Rect {
    let (width, height) = size;
    let (image_width, image_height) = image;
    if fit == ImageFit::Stretch || image_width <= 0.0 || image_height <= 0.0 {
        return Rect::new(0.0, 0.0, width, height);
    }
    let (across, down) = (width / image_width, height / image_height);
    let scale = match fit {
        ImageFit::Cover => across.max(down),
        _ => across.min(down),
    };
    let (fitted_width, fitted_height) = (image_width * scale, image_height * scale);
    Rect::new(
        (width - fitted_width) / 2.0,
        (height - fitted_height) / 2.0,
        fitted_width,
        fitted_height,
    )
}

impl Converter<'_> {
    /// Adds an image layer: its picture, stretched to the box, cut to its
    /// corners, with the box's outline above it.
    pub fn add_image(
        &mut self,
        node: &Node,
        image: &ImageLayer,
        comp: &mut Comp,
        parent_index: Option<i64>,
        offset: [f64; 2],
    ) -> Result<(), ExportError> {
        let written = match self.images.asset(&image.source, &mut self.assets) {
            Ok(written) => written,
            Err(reason) => {
                self.warnings.layer(
                    node,
                    &format!("is left out: its picture cannot be used ({reason})."),
                );
                return Ok(());
            }
        };
        let id = &node.id;
        let hints = self.timeline.hints(id);
        let (picture_width, picture_height) = (
            f64::from(written.size.0.max(1)),
            f64::from(written.size.1.max(1)),
        );
        // The outline is drawn above the picture, by a layer of its own.
        let outline = self.paint_items(node, &hints, Parts::STROKE);
        if !outline.is_empty() {
            let box_transform = self.transform(node, &hints, offset);
            let index = comp.index();
            let mut layer = self.layer(node, 4, index, parent_index, &box_transform);
            layer.insert("nm".into(), Value::from(format!("{} outline", node.name())));
            layer.insert("shapes".into(), Value::Array(outline));
            self.add_masks(node, &mut layer, &|_| Place::default(), Masks::REVEAL);
            comp.layers.push(Value::Object(layer));
        }

        // The picture's pixels to the box: a scale each way and a shift.
        let fit = image.fit;
        let placement = |frame: f64| {
            let size = self.size(id, frame);
            let target = fitted(fit, size, (picture_width, picture_height));
            (
                [target.width / picture_width, target.height / picture_height],
                [target.x, target.y],
            )
        };
        let mut transform = self.transform(node, &hints, offset);
        let timeline = self.timeline;
        let length = places_for(tolerance::LENGTH);
        let anchor = |frame: f64| {
            let (width, height) = self.size(id, frame);
            let (scale, shift) = placement(frame);
            let inverse = |value: f64, scale: f64| if scale > 0.0 { value / scale } else { 0.0 };
            vec![
                inverse(width / 2.0 - shift[0], scale[0]),
                inverse(height / 2.0 - shift[1], scale[1]),
            ]
        };
        let scale = |frame: f64| {
            let layer = timeline.number(id, Property::Scale, frame) * 100.0;
            let (scale, _) = placement(frame);
            vec![layer * scale[0], layer * scale[1]]
        };
        transform.anchor = timeline.fit(&anchor, &hints, tolerance::LENGTH / 10.0, length + 1);
        transform.scale = timeline.fit(&scale, &hints, tolerance::SCALE / 10.0, 5);

        let index = comp.index();
        let mut layer = self.layer(node, 2, index, parent_index, &transform);
        layer.insert("refId".into(), Value::from(written.id.as_str()));
        // A picture that reaches past its box, or a box with round
        // corners, is cut to the box.
        let radius = self.survey.iter().any(|frame| {
            timeline
                .radii(id, *frame)
                .iter()
                .any(|radius| *radius > 0.0)
        });
        let masks = Masks {
            clip: radius || fit == ImageFit::Cover,
            reveal: true,
        };
        let place = |frame: f64| {
            let (scale, shift) = placement(frame);
            let inverse = |scale: f64| if scale > 0.0 { 1.0 / scale } else { 0.0 };
            Place {
                scale: [inverse(scale[0]), inverse(scale[1])],
                offset: [-shift[0] * inverse(scale[0]), -shift[1] * inverse(scale[1])],
            }
        };
        self.add_masks(node, &mut layer, &place, masks);
        comp.layers.push(Value::Object(layer));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::new_rgba8(width, height);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn png_and_jpeg_are_kept_as_they_are() {
        let bytes = png(3, 2);
        let read = picture(bytes.clone()).unwrap();
        assert_eq!(
            (read.bytes, read.format, read.size),
            (bytes, ImageFormat::Png, (3, 2))
        );
        let mut gif = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(4, 5)
            .write_to(&mut gif, ImageFormat::Gif)
            .unwrap();
        let read = picture(gif.into_inner()).unwrap();
        assert_eq!((read.format, read.size), (ImageFormat::Png, (4, 5)));
        assert!(read.bytes.starts_with(b"\x89PNG"));
        assert!(picture(b"not a picture".to_vec()).is_err());
    }

    #[test]
    fn pictures_fit_their_box_like_the_renderer_fits_them() {
        let cover = fitted(ImageFit::Cover, (100.0, 100.0), (200.0, 100.0));
        assert_eq!(cover, Rect::new(-50.0, 0.0, 200.0, 100.0));
        let contain = fitted(ImageFit::Contain, (100.0, 100.0), (200.0, 100.0));
        assert_eq!(contain, Rect::new(0.0, 25.0, 100.0, 50.0));
        let stretch = fitted(ImageFit::Stretch, (100.0, 40.0), (200.0, 100.0));
        assert_eq!(stretch, Rect::new(0.0, 0.0, 100.0, 40.0));
    }

    #[test]
    fn files_beside_appear_only_when_committed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("clip_images");
        let mut files = ImageFiles::default();
        files.add(&folder, "image_0.png", b"png").unwrap();
        assert_eq!(
            fs::read_dir(&folder).unwrap().count(),
            1,
            "a temporary file"
        );
        assert!(!folder.join("image_0.png").exists());
        files.commit().unwrap();
        assert_eq!(fs::read(folder.join("image_0.png")).unwrap(), b"png");

        // An export that stops leaves nothing, not even the folder.
        let other = dir.path().join("other_images");
        let mut files = ImageFiles::default();
        files.add(&other, "image_0.png", b"png").unwrap();
        drop(files);
        assert!(!other.exists());
    }
}
