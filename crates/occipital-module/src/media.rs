//! Images in a chat, as the apps draw them.
//!
//! A page's image file parts carry the picture's width and height, read once
//! from the image's first bytes through the runner contract's
//! `session.media_read`, so an app can lay out a row before any picture arrives
//! ([`Sizes`]). `session.media` answers a thumbnail this module makes from the
//! runner's bytes, or the original bytes in pieces ([`answer`]). The runner
//! contract leaves thumbnails to the reader: runners keep only the original.
//!
//! Sizes and thumbnails follow the picture's EXIF orientation, so a photo taken
//! sideways reports its upright size and its thumbnail comes upright. The
//! original bytes are passed on unchanged, and a viewer applies the rotation
//! itself.
//!
//! Both caches live in memory only. A size costs one header read to learn again
//! after a restart, and the apps keep thumbnails by `sha256` on their side.

use std::{
    collections::{HashMap, VecDeque},
    io::Cursor,
    sync::Mutex,
};

use image::{
    codecs::jpeg::JpegEncoder, metadata::Orientation, DynamicImage, ImageDecoder, ImageError,
    ImageFormat, ImageReader, Limits,
};
use occipital_app_reader::source::ReadError;
use serde_json::{json, Value};

use crate::app_reader::{Guard, SourceResolver};

pub const OPERATION: &str = "session.media";
/// The thumbnail's longest side, in pixels: a phone bubble about 240 points wide
/// is 720 pixels at the phone's 3x scale, and the desktop scales it down.
pub const THUMB_SIDE: u32 = 1024;
/// The largest piece of the original one `full` reply carries.
pub const PIECE_MAX: u64 = 1 << 20;
/// The largest image this module decodes for a thumbnail, by file size and by
/// pixels. Decoding needs about four bytes per pixel, so the pixel cap bounds the
/// memory one thumbnail can take.
pub const MAX_SOURCE_BYTES: u64 = 32 << 20;
pub const MAX_PIXELS: u64 = 50_000_000;
/// The first bytes read to learn an image's size. JPEG keeps its size after the
/// EXIF block, which a camera can fill with its own small preview.
pub const HEADER_BYTES: u64 = 256 << 10;
const JPEG_QUALITY: u8 = 80;
/// Remembered sizes, and thumbnail bytes kept, before the oldest are dropped.
const KNOWN_SIZES: usize = 8_192;
const THUMB_BYTES: usize = 64 << 20;

pub mod codes {
    /// The file isn't an image this module decodes, or its bytes don't decode.
    pub const MEDIA_NOT_IMAGE: &str = "media_not_image";
    /// The image is above [`super::MAX_SOURCE_BYTES`] or [`super::MAX_PIXELS`].
    pub const MEDIA_TOO_LARGE_TO_THUMB: &str = "media_too_large_to_thumb";
}

/// A picture's size as it is drawn, after its EXIF orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// Whether `orientation` turns the picture a quarter, so its drawn width is its
/// encoded height.
fn turns(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

/// The drawn size of the image whose file starts with `prefix`, or `None` when
/// those bytes aren't the start of a PNG, JPEG, WebP or GIF, or don't hold its
/// header. An orientation the bytes don't reach is taken as none.
pub fn header_size(prefix: &[u8]) -> Option<Size> {
    let reader = ImageReader::new(Cursor::new(prefix))
        .with_guessed_format()
        .ok()?;
    reader.format()?;
    let mut decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    Some(if turns(orientation) {
        Size {
            width: height,
            height: width,
        }
    } else {
        Size { width, height }
    })
}

/// A thumbnail: JPEG, or PNG when any pixel is transparent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumb {
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbRefusal {
    NotImage,
    TooLarge,
}

impl ThumbRefusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotImage => codes::MEDIA_NOT_IMAGE,
            Self::TooLarge => codes::MEDIA_TOO_LARGE_TO_THUMB,
        }
    }
}

/// The thumbnail of the image `bytes` hold: upright, its longest side at most
/// [`THUMB_SIDE`], never larger than the original.
pub fn thumbnail(bytes: &[u8]) -> Result<Thumb, ThumbRefusal> {
    thumbnail_within(bytes, MAX_PIXELS)
}

/// [`thumbnail`], refusing images above `max_pixels` before decoding them.
fn thumbnail_within(bytes: &[u8], max_pixels: u64) -> Result<Thumb, ThumbRefusal> {
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err(ThumbRefusal::TooLarge);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ThumbRefusal::NotImage)?;
    if reader.format().is_none() {
        return Err(ThumbRefusal::NotImage);
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(max_pixels.saturating_mul(4));
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| ThumbRefusal::NotImage)?;
    let (width, height) = decoder.dimensions();
    // The header names the size before any pixel is decoded, so a picture too
    // large to hold is refused before its memory is taken.
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(ThumbRefusal::TooLarge);
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(|error| match error {
        ImageError::Limits(_) => ThumbRefusal::TooLarge,
        _ => ThumbRefusal::NotImage,
    })?;
    image.apply_orientation(orientation);
    if image.width().max(image.height()) > THUMB_SIDE {
        image = image.thumbnail(THUMB_SIDE, THUMB_SIDE);
    }
    let transparent =
        image.color().has_alpha() && image.to_rgba8().pixels().any(|pixel| pixel.0[3] < u8::MAX);
    let mut out = Vec::new();
    let mime = if transparent {
        image
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .map_err(|_| ThumbRefusal::NotImage)?;
        "image/png"
    } else {
        JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
            .encode_image(&DynamicImage::ImageRgb8(image.to_rgb8()))
            .map_err(|_| ThumbRefusal::NotImage)?;
        "image/jpeg"
    };
    Ok(Thumb {
        mime,
        width: image.width(),
        height: image.height(),
        bytes: out,
    })
}

/// A map that forgets its oldest entries once it holds `cap` of them, or `cap`
/// bytes when each entry is weighed.
struct Bounded<V> {
    entries: HashMap<String, (V, usize)>,
    order: VecDeque<String>,
    held: usize,
    cap: usize,
}

impl<V: Clone> Bounded<V> {
    fn new(cap: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            held: 0,
            cap,
        }
    }

    fn get(&self, key: &str) -> Option<V> {
        self.entries.get(key).map(|(value, _)| value.clone())
    }

    fn put(&mut self, key: &str, value: V, weight: usize) {
        if let Some((_, old)) = self.entries.insert(key.to_owned(), (value, weight)) {
            self.held -= old;
        } else {
            self.order.push_back(key.to_owned());
        }
        self.held += weight;
        while self.held > self.cap {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some((_, weight)) = self.entries.remove(&oldest) {
                self.held -= weight;
            }
        }
    }
}

/// The drawn sizes of images, by `sha256`, as runners have served them.
pub struct Sizes {
    known: Mutex<Bounded<Option<Size>>>,
}

impl Default for Sizes {
    fn default() -> Self {
        Self {
            known: Mutex::new(Bounded::new(KNOWN_SIZES)),
        }
    }
}

impl Sizes {
    /// `Some(None)` when the image's first bytes were read and held no size this
    /// module reads, so they are not read again; `None` when they were never read.
    pub fn get(&self, sha256: &str) -> Option<Option<Size>> {
        self.known.guard().get(sha256)
    }

    pub fn put(&self, sha256: &str, size: Option<Size>) {
        self.known.guard().put(sha256, size, 1);
    }

    /// The digests of the images in a row's `display` that have no size and
    /// whose first bytes haven't been read, in the row's order.
    pub fn unknown(&self, display: &Value) -> Vec<String> {
        let mut out = Vec::new();
        for part in image_parts(display) {
            if let Some(sha256) = part["sha256"].as_str() {
                if self.get(sha256).is_none() && !out.iter().any(|seen| seen == sha256) {
                    out.push(sha256.to_owned());
                }
            }
        }
        out
    }

    /// Fill in `width` and `height` on each image of a row's `display`, in its
    /// parts and its files list, whose size is known.
    pub fn apply(&self, display: &mut Value) {
        for key in ["parts", "files"] {
            let Some(parts) = display.get_mut(key).and_then(Value::as_array_mut) else {
                continue;
            };
            for part in parts.iter_mut().filter(|part| is_unsized_image(part)) {
                let known = part["sha256"].as_str().and_then(|sha256| self.get(sha256));
                if let Some(Some(size)) = known {
                    part["width"] = json!(size.width);
                    part["height"] = json!(size.height);
                }
            }
        }
    }
}

/// An image file without a size: a file part, or an entry of the files list,
/// whose type is an image and whose digest is known.
fn is_unsized_image(part: &Value) -> bool {
    part["type"].as_str().is_none_or(|kind| kind == "file")
        && part["mime"]
            .as_str()
            .is_some_and(|mime| mime.starts_with("image/"))
        && part["sha256"].is_string()
        && (part.get("width").is_none() || part.get("height").is_none())
}

fn image_parts(display: &Value) -> impl Iterator<Item = &Value> {
    ["parts", "files"]
        .into_iter()
        .filter_map(|key| display.get(key).and_then(Value::as_array))
        .flatten()
        .filter(|part| is_unsized_image(part))
}

/// Thumbnails made so far, and the one decode allowed at a time: a decode can
/// take [`MAX_PIXELS`] times four bytes, so they run one by one.
pub struct Thumbs {
    made: Mutex<Bounded<Thumb>>,
    decoding: tokio::sync::Semaphore,
}

impl Default for Thumbs {
    fn default() -> Self {
        Self {
            made: Mutex::new(Bounded::new(THUMB_BYTES)),
            decoding: tokio::sync::Semaphore::new(1),
        }
    }
}

/// A `session.media` request.
struct MediaRequest {
    session: String,
    harness: Option<String>,
    sha256: String,
    full: bool,
    offset: u64,
    length: u64,
}

fn invalid(field: &str, message: &str) -> ReadError {
    ReadError::new("invalid_params", message).with_detail(json!({"field": field}))
}

impl MediaRequest {
    fn parse(params: &Value) -> Result<Self, ReadError> {
        let session = params["session"]
            .as_str()
            .filter(|session| !session.is_empty())
            .ok_or_else(|| invalid("session", "session is missing or malformed"))?;
        let harness = occipital_app_reader::page::optional_transcript_harness(params)
            .map_err(|error| ReadError::new(error.code, error.message))?;
        let sha256 = params["sha256"]
            .as_str()
            .filter(|sha256| {
                sha256.len() == 64
                    && sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
            .ok_or_else(|| invalid("sha256", "sha256 must be 64 lowercase hex characters"))?;
        let full = match params["size"].as_str() {
            Some("thumb") => false,
            Some("full") => true,
            _ => return Err(invalid("size", "size must be \"thumb\" or \"full\"")),
        };
        let number = |field: &str, default: u64| match params.get(field) {
            None => Ok(default),
            Some(value) => value
                .as_u64()
                .ok_or_else(|| invalid(field, &format!("{field} must be a whole number"))),
        };
        let (offset, length) = if full {
            (number("offset", 0)?, number("length", PIECE_MAX)?)
        } else {
            if params.get("offset").is_some() || params.get("length").is_some() {
                return Err(invalid(
                    "size",
                    "offset and length are only for size \"full\"",
                ));
            }
            (0, 0)
        };
        if full && !(1..=PIECE_MAX).contains(&length) {
            return Err(invalid(
                "length",
                &format!("length must be 1 to {PIECE_MAX} bytes"),
            ));
        }
        Ok(Self {
            session: session.to_owned(),
            harness: harness.map(str::to_owned),
            sha256: sha256.to_owned(),
            full,
            offset,
            length,
        })
    }
}

/// The runner's `session.media_read` reply, its piece decoded.
pub struct MediaPiece {
    pub mime: String,
    pub size: u64,
    pub bytes: Vec<u8>,
}

/// Decode a `session.media_read` reply.
pub fn media_piece(value: Value) -> Result<MediaPiece, ReadError> {
    let reply: cortexkit_role_llm_runner::attachments::MediaReadReply =
        serde_json::from_value(value).map_err(|error| {
            ReadError::new(
                "invalid_provider_reply",
                format!("session.media_read: {error}"),
            )
        })?;
    let bytes = match reply.data() {
        None => Vec::new(),
        Some(data) => crate::base64::decode(data).ok_or_else(|| {
            ReadError::new(
                "invalid_provider_reply",
                "session.media_read sent data that isn't canonical base64",
            )
        })?,
    };
    Ok(MediaPiece {
        mime: reply.mime().to_owned(),
        size: reply.size(),
        bytes,
    })
}

/// Answer `session.media`: a thumbnail, `{mime, width, height, data}`, or a piece
/// of the original, `{mime, size, offset, data}`, with `data` in standard base64.
pub async fn answer(
    params: &Value,
    resolver: &dyn SourceResolver,
    thumbs: &Thumbs,
) -> Result<Value, ReadError> {
    let request = MediaRequest::parse(params)?;
    let read = |offset: u64, length: u64| {
        resolver.media_read(
            &request.session,
            request.harness.as_deref(),
            &request.sha256,
            offset,
            length,
        )
    };
    if request.full {
        let piece = media_piece(read(request.offset, request.length).await?)?;
        return Ok(json!({
            "mime": piece.mime, "size": piece.size, "offset": request.offset,
            "data": crate::base64::encode(&piece.bytes),
        }));
    }
    // The runner admits the read first: a digest this chat doesn't reference is
    // refused there, even when another chat's thumbnail is already made.
    let probe = media_piece(read(0, 0).await?)?;
    if let Some(thumb) = thumbs.made.guard().get(&request.sha256) {
        return Ok(thumb_reply(&thumb));
    }
    if !probe.mime.starts_with("image/") {
        return Err(ReadError::new(
            codes::MEDIA_NOT_IMAGE,
            format!("{} isn't an image", probe.mime),
        ));
    }
    if probe.size > MAX_SOURCE_BYTES {
        return Err(ReadError::new(
            codes::MEDIA_TOO_LARGE_TO_THUMB,
            format!("the image is above {MAX_SOURCE_BYTES} bytes"),
        ));
    }
    let mut bytes = Vec::with_capacity(probe.size as usize);
    while (bytes.len() as u64) < probe.size {
        let offset = bytes.len() as u64;
        let length =
            (probe.size - offset).min(cortexkit_role_llm_runner::attachments::CHUNK_MAX as u64);
        let piece = media_piece(read(offset, length).await?)?;
        if piece.bytes.is_empty() || piece.size != probe.size {
            return Err(ReadError::new(
                "invalid_provider_reply",
                "session.media_read changed the image's size or stopped short",
            ));
        }
        bytes.extend_from_slice(&piece.bytes);
    }
    if bytes.len() as u64 != probe.size {
        return Err(ReadError::new(
            "invalid_provider_reply",
            "session.media_read sent more bytes than the image holds",
        ));
    }
    let _permit = thumbs
        .decoding
        .acquire()
        .await
        .map_err(|_| ReadError::new("provider_unavailable", "thumbnails are shutting down"))?;
    let thumb = tokio::task::spawn_blocking(move || thumbnail(&bytes))
        .await
        .map_err(|_| {
            ReadError::new(
                codes::MEDIA_NOT_IMAGE,
                "the image's decoder failed on its bytes",
            )
        })?
        .map_err(|refusal| ReadError::new(refusal.code(), "no thumbnail for this image"))?;
    let reply = thumb_reply(&thumb);
    let weight = thumb.bytes.len();
    thumbs.made.guard().put(&request.sha256, thumb, weight);
    Ok(reply)
}

fn thumb_reply(thumb: &Thumb) -> Value {
    json!({
        "mime": thumb.mime, "width": thumb.width, "height": thumb.height,
        "data": crate::base64::encode(&thumb.bytes),
    })
}

#[cfg(test)]
mod tests;
