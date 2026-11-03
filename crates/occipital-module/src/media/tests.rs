use super::*;
use async_trait::async_trait;
use image::{Rgb, RgbImage, Rgba, RgbaImage};
use occipital_app_reader::source::TranscriptSource;
use sha2::{Digest, Sha256};
use std::sync::Arc;

fn png(image: DynamicImage) -> Vec<u8> {
    let mut out = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 90)
        .encode_image(&DynamicImage::ImageRgb8(RgbImage::from_pixel(
            width,
            height,
            Rgb([200, 40, 40]),
        )))
        .unwrap();
    out
}

/// `jpeg` with an EXIF block saying how to turn it, as a camera writes one: a
/// big-endian TIFF header and one image file directory (IFD) entry for
/// Orientation (0x0112), right after the file's start-of-image marker.
fn with_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut exif = b"Exif\0\0MM\0*\0\0\0\x08".to_vec();
    exif.extend(1u16.to_be_bytes());
    exif.extend(0x0112u16.to_be_bytes());
    exif.extend(3u16.to_be_bytes());
    exif.extend(1u32.to_be_bytes());
    exif.extend(orientation.to_be_bytes());
    exif.extend([0, 0]);
    exif.extend(0u32.to_be_bytes());
    let mut out = jpeg[..2].to_vec();
    out.extend([0xFF, 0xE1]);
    out.extend(((exif.len() + 2) as u16).to_be_bytes());
    out.extend(exif);
    out.extend(&jpeg[2..]);
    out
}

fn opaque(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb([10, 120, 200])))
}

#[test]
fn a_size_comes_from_the_first_bytes_and_follows_the_exif_rotation() {
    assert_eq!(
        header_size(&png(opaque(300, 120))),
        Some(Size {
            width: 300,
            height: 120
        })
    );
    let plain = jpeg(64, 32);
    assert_eq!(
        header_size(&plain),
        Some(Size {
            width: 64,
            height: 32
        })
    );
    // Turned a quarter (6) or three quarters (8), the photo is drawn portrait;
    // upside down (3) it keeps its shape.
    for (orientation, size) in [(6, (32, 64)), (8, (32, 64)), (3, (64, 32)), (1, (64, 32))] {
        let turned = with_orientation(&plain, orientation);
        assert_eq!(
            header_size(&turned),
            Some(Size {
                width: size.0,
                height: size.1
            }),
            "orientation {orientation}"
        );
        // Only the headers are needed: everything up to the start of the
        // compressed picture (the SOS marker), not the picture itself.
        let scan = turned
            .windows(2)
            .position(|marker| marker == [0xFF, 0xDA])
            .unwrap();
        assert_eq!(
            header_size(&turned[..scan + 16]).map(|size| (size.width, size.height)),
            Some(size),
            "orientation {orientation}, headers only"
        );
    }
    for bytes in [&b"%PDF-1.7\n"[..], b"", b"\x89PNG\r\n"] {
        assert_eq!(header_size(bytes), None, "{bytes:?}");
    }
}

#[test]
fn a_thumbnail_is_upright_at_most_1024_and_never_upscaled() {
    let wide = thumbnail(&png(opaque(3000, 1500))).unwrap();
    assert_eq!(
        (wide.mime, wide.width, wide.height),
        ("image/jpeg", 1024, 512)
    );
    assert_eq!(
        header_size(&wide.bytes),
        Some(Size {
            width: 1024,
            height: 512
        })
    );
    let small = thumbnail(&jpeg(100, 50)).unwrap();
    assert_eq!((small.width, small.height), (100, 50));
    // A photo taken sideways comes upright.
    let turned = thumbnail(&with_orientation(&jpeg(2000, 1000), 6)).unwrap();
    assert_eq!((turned.width, turned.height), (512, 1024));
}

#[test]
fn a_thumbnail_keeps_transparency_as_png_and_opaque_pixels_as_jpeg() {
    let mut picture = RgbaImage::from_pixel(40, 40, Rgba([0, 0, 0, 255]));
    let solid = thumbnail(&png(DynamicImage::ImageRgba8(picture.clone()))).unwrap();
    assert_eq!(
        solid.mime, "image/jpeg",
        "an alpha channel that's all opaque"
    );
    picture.put_pixel(3, 3, Rgba([0, 0, 0, 0]));
    let clear = thumbnail(&png(DynamicImage::ImageRgba8(picture))).unwrap();
    assert_eq!(clear.mime, "image/png");
}

#[test]
fn files_that_arent_images_or_are_too_large_are_refused() {
    assert_eq!(thumbnail(b"%PDF-1.7\n"), Err(ThumbRefusal::NotImage));
    let mut truncated = jpeg(64, 64);
    truncated.truncate(truncated.len() / 3);
    assert_eq!(thumbnail(&truncated), Err(ThumbRefusal::NotImage));
    let picture = png(opaque(200, 100));
    assert_eq!(
        thumbnail_within(&picture, 200 * 100 - 1),
        Err(ThumbRefusal::TooLarge)
    );
    assert!(thumbnail_within(&picture, 200 * 100).is_ok());
}

#[test]
fn sizes_fill_in_each_unsized_image_in_parts_and_files() {
    let sizes = Sizes::default();
    let image = |sha256: &str| json!({"type": "file", "mime": "image/png", "sha256": sha256});
    let mut display = json!({
        "parts": [
            image("aa"),
            {"type": "file", "mime": "application/pdf", "sha256": "bb"},
            {"type": "file", "mime": "image/png", "sha256": "cc", "width": 9, "height": 9},
            {"type": "text", "text": "hi"},
        ],
        "files": [{"mime": "image/png", "sha256": "aa"}],
    });
    assert_eq!(sizes.unknown(&display), ["aa"]);
    sizes.put(
        "aa",
        Some(Size {
            width: 30,
            height: 20,
        }),
    );
    assert!(sizes.unknown(&display).is_empty());
    sizes.apply(&mut display);
    assert_eq!(
        (
            &display["parts"][0]["width"],
            &display["parts"][0]["height"]
        ),
        (&json!(30), &json!(20))
    );
    assert_eq!(display["files"][0]["width"], 30);
    assert!(display["parts"][1].get("width").is_none());
    assert_eq!(display["parts"][2]["width"], 9);
    // An image whose bytes held no size is remembered, not read again, and
    // keeps no size.
    let mut unread = json!({"parts": [image("dd")]});
    sizes.put("dd", None);
    assert!(sizes.unknown(&unread).is_empty());
    sizes.apply(&mut unread);
    assert!(unread["parts"][0].get("width").is_none());
}

/// A session whose runner serves the files it holds by digest, as
/// `session.media_read` does, counting its reads.
#[derive(Default)]
struct Held {
    files: Mutex<HashMap<String, (String, Vec<u8>)>>,
    /// Sizes claimed in place of the real ones, by digest.
    claimed: Mutex<HashMap<String, u64>>,
    reads: Mutex<Vec<(u64, u64)>>,
}

impl Held {
    fn hold(&self, mime: &str, bytes: &[u8]) -> String {
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        self.files
            .guard()
            .insert(sha256.clone(), (mime.into(), bytes.to_vec()));
        sha256
    }
}

#[async_trait]
impl SourceResolver for Held {
    async fn resolve(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError> {
        Ok(None)
    }

    async fn media_read(
        &self,
        session: &str,
        harness: Option<&str>,
        sha256: &str,
        offset: u64,
        length: u64,
    ) -> Result<Value, ReadError> {
        assert_eq!((session, harness), ("s", Some("opencode2")));
        self.reads.guard().push((offset, length));
        let files = self.files.guard();
        let Some((mime, bytes)) = files.get(sha256) else {
            return Err(ReadError::new("unknown_media", "not in this session"));
        };
        let size = self
            .claimed
            .guard()
            .get(sha256)
            .copied()
            .unwrap_or(bytes.len() as u64);
        let mut reply = json!({"size": size, "mime": mime});
        if length > 0 {
            let start = (offset as usize).min(bytes.len());
            let end = (start + length as usize).min(bytes.len());
            reply["data"] = json!(crate::base64::encode(&bytes[start..end]));
        }
        Ok(reply)
    }
}

fn media(sha256: &str, size: &str) -> Value {
    json!({"session": "s", "harness": "opencode2", "sha256": sha256, "size": size})
}

#[tokio::test]
async fn a_thumbnail_is_read_whole_made_once_and_kept() {
    let held = Held::default();
    let picture = png(opaque(2048, 1024));
    let sha256 = held.hold("image/png", &picture);
    let thumbs = Thumbs::default();
    let reply = answer(&media(&sha256, "thumb"), &held, &thumbs)
        .await
        .unwrap();
    assert_eq!(
        (&reply["mime"], &reply["width"], &reply["height"]),
        (&json!("image/jpeg"), &json!(1024), &json!(512))
    );
    let bytes = crate::base64::decode(reply["data"].as_str().unwrap()).unwrap();
    assert_eq!(
        header_size(&bytes),
        Some(Size {
            width: 1024,
            height: 512
        })
    );
    assert_eq!(held.reads.guard()[0], (0, 0), "a probe first");
    let reads = held.reads.guard().len();
    // The second ask is answered from the thumbnail made, after the runner
    // admits the digest again.
    let again = answer(&media(&sha256, "thumb"), &held, &thumbs)
        .await
        .unwrap();
    assert_eq!(again, reply);
    assert_eq!(held.reads.guard().len(), reads + 1);
}

#[tokio::test]
async fn the_original_comes_in_pieces_of_at_most_a_mebibyte() {
    let held = Held::default();
    let bytes: Vec<u8> = (0..(PIECE_MAX as usize + 10))
        .map(|i| (i % 251) as u8)
        .collect();
    let sha256 = held.hold("image/jpeg", &bytes);
    let thumbs = Thumbs::default();
    let first = answer(&media(&sha256, "full"), &held, &thumbs)
        .await
        .unwrap();
    assert_eq!(
        (&first["mime"], &first["size"], &first["offset"]),
        (&json!("image/jpeg"), &json!(bytes.len()), &json!(0))
    );
    let piece = crate::base64::decode(first["data"].as_str().unwrap()).unwrap();
    assert_eq!(piece, bytes[..PIECE_MAX as usize]);
    let mut rest = media(&sha256, "full");
    rest["offset"] = json!(PIECE_MAX);
    let last = answer(&rest, &held, &thumbs).await.unwrap();
    assert_eq!(
        crate::base64::decode(last["data"].as_str().unwrap()).unwrap(),
        bytes[PIECE_MAX as usize..]
    );
    let mut large = media(&sha256, "full");
    large["length"] = json!(PIECE_MAX + 1);
    assert_eq!(
        answer(&large, &held, &thumbs).await.unwrap_err().code,
        "invalid_params"
    );
}

#[tokio::test]
async fn refusals_name_why_no_thumbnail_comes() {
    let held = Held::default();
    let thumbs = Thumbs::default();
    let pdf = held.hold("application/pdf", b"%PDF-1.7\n");
    let broken = held.hold("image/png", b"not a png at all");
    let huge = held.hold("image/png", &png(opaque(8, 8)));
    held.claimed
        .guard()
        .insert(huge.clone(), MAX_SOURCE_BYTES + 1);
    let unknown = "0".repeat(64);
    for (sha256, code) in [
        (&pdf, codes::MEDIA_NOT_IMAGE),
        (&broken, codes::MEDIA_NOT_IMAGE),
        (&huge, codes::MEDIA_TOO_LARGE_TO_THUMB),
        (&unknown, "unknown_media"),
    ] {
        let refusal = answer(&media(sha256, "thumb"), &held, &thumbs)
            .await
            .unwrap_err();
        assert_eq!(refusal.code, code, "{sha256}");
    }
    // Only the probe was read for the file too large to decode.
    assert!(held
        .reads
        .guard()
        .iter()
        .all(|(_, length)| *length < 1 << 20));
    for (params, field) in [
        (
            json!({"session": "s", "sha256": pdf, "size": "huge"}),
            "size",
        ),
        (
            json!({"session": "s", "sha256": "ABC", "size": "thumb"}),
            "sha256",
        ),
        (json!({"sha256": pdf, "size": "thumb"}), "session"),
        (
            json!({"session": "s", "sha256": pdf, "size": "thumb", "offset": 4}),
            "size",
        ),
    ] {
        let refusal = answer(&params, &held, &thumbs).await.unwrap_err();
        assert_eq!(
            (refusal.code, &refusal.detail),
            ("invalid_params", &Some(json!({"field": field}))),
            "{params}"
        );
    }
}
