//! Getting a file ready to send (#57): images become WebP here, on the sender's machine, so the
//! server never decodes anyone's image. Everything else goes as it is.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use pulse_protocol::rest::AttachmentKind;

/// An image's longest side after conversion **(owner)**.
pub const MAX_SIDE: u32 = 4096;
/// libwebp's lossy quality: high, a screenshot's text stays crisp.
pub const QUALITY: f32 = 90.0;
/// The thumbnail the composer shows on a queued image's chip.
pub const PREVIEW_SIDE: u32 = 96;
/// Every frame of an animation is held decoded at once: past this many pixels in all (about 1 GB
/// of RGBA) it is sent as the original file instead of converted.
const MAX_ANIMATION_PIXELS: u64 = 256 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum PrepareError {
    /// Not an image this can read (or a broken one): sent as a plain file.
    NotAnImage,
    /// An animation too big to convert: sent as a plain file.
    TooBig,
    Encode,
}

/// A converted image.
pub struct WebpOut {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    /// A small PNG of the first frame, for the composer's chip.
    pub preview_png: Vec<u8>,
}

/// File names the composer converts (by extension; the contents decide in the end).
pub fn is_image_name(name: &str) -> bool {
    matches!(
        extension(name).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
    )
}

fn extension(name: &str) -> Option<String> {
    let (_, ext) = name.rsplit_once('.')?;
    Some(ext.to_ascii_lowercase())
}

/// The content type a file is declared with, by extension, and its kind. The server keeps media
/// types it knows and stores everything else as a plain file. Images are declared only after
/// conversion (`image/webp`), so an image name here (one that failed to convert) is a file.
pub fn content_type_for(name: &str) -> (&'static str, AttachmentKind) {
    use AttachmentKind::*;
    match extension(name).as_deref() {
        Some("mp4" | "m4v") => ("video/mp4", Video),
        Some("webm") => ("video/webm", Video),
        Some("mov") => ("video/quicktime", Video),
        Some("mp3") => ("audio/mpeg", Audio),
        Some("ogg" | "oga" | "opus") => ("audio/ogg", Audio),
        Some("wav") => ("audio/wav", Audio),
        Some("flac") => ("audio/flac", Audio),
        Some("aac") => ("audio/aac", Audio),
        Some("m4a") => ("audio/mp4", Audio),
        _ => ("application/octet-stream", File),
    }
}

/// `photo.PNG` → `photo.webp`.
pub fn webp_name(name: &str) -> String {
    let stem = match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    };
    format!("{stem}.webp")
}

/// The size an image of `w`×`h` is sent at: unchanged up to [`MAX_SIDE`], else scaled down to it.
fn fit(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= MAX_SIDE {
        return (w, h);
    }
    let scale = |v: u32| ((v as u64 * MAX_SIDE as u64) / long as u64).max(1) as u32;
    (scale(w), scale(h))
}

fn resize(img: RgbaImage, nw: u32, nh: u32) -> RgbaImage {
    if img.dimensions() == (nw, nh) {
        return img;
    }
    DynamicImage::ImageRgba8(img)
        .resize_exact(nw, nh, FilterType::Lanczos3)
        .into_rgba8()
}

/// Every frame (with its delay in ms) of an animated GIF or WebP; `None` for a still.
fn animation(
    bytes: &[u8],
    format: ImageFormat,
) -> Result<Option<Vec<(RgbaImage, i32)>>, PrepareError> {
    let frames = match format {
        ImageFormat::Gif => GifDecoder::new(Cursor::new(bytes))
            .map_err(|_| PrepareError::NotAnImage)?
            .into_frames(),
        ImageFormat::WebP => {
            let d = WebPDecoder::new(Cursor::new(bytes)).map_err(|_| PrepareError::NotAnImage)?;
            if !d.has_animation() {
                return Ok(None);
            }
            d.into_frames()
        }
        _ => return Ok(None),
    };
    let mut out = Vec::new();
    let mut pixels: u64 = 0;
    for f in frames {
        let f = f.map_err(|_| PrepareError::NotAnImage)?;
        let (num, den) = f.delay().numer_denom_ms();
        let ms = if den == 0 { 100 } else { (num / den) as i32 };
        let img = f.into_buffer();
        pixels += img.width() as u64 * img.height() as u64;
        if pixels > MAX_ANIMATION_PIXELS {
            return Err(PrepareError::TooBig);
        }
        out.push((img, ms.max(20)));
    }
    // A one-frame GIF is a still.
    Ok((out.len() > 1).then_some(out))
}

/// The image in `bytes` as WebP: the longest side at most [`MAX_SIDE`], an animation keeping every
/// frame and its timing, a phone photo turned the way its EXIF says.
pub fn to_webp(bytes: &[u8]) -> Result<WebpOut, PrepareError> {
    let format = image::guess_format(bytes).map_err(|_| PrepareError::NotAnImage)?;
    if !matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Bmp
    ) {
        return Err(PrepareError::NotAnImage);
    }
    let frames = match animation(bytes, format)? {
        Some(frames) => {
            let (w, h) = frames[0].0.dimensions();
            let (nw, nh) = fit(w, h);
            frames
                .into_iter()
                .map(|(img, ms)| (resize(img, nw, nh), ms))
                .collect::<Vec<_>>()
        }
        None => {
            let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
                .into_decoder()
                .map_err(|_| PrepareError::NotAnImage)?;
            let orientation = decoder.orientation().ok();
            let mut img =
                DynamicImage::from_decoder(decoder).map_err(|_| PrepareError::NotAnImage)?;
            if let Some(o) = orientation {
                img.apply_orientation(o);
            }
            let (nw, nh) = fit(img.width(), img.height());
            vec![(resize(img.into_rgba8(), nw, nh), 100)]
        }
    };
    let (width, height) = frames[0].0.dimensions();
    let preview_png = preview(&frames[0].0)?;
    let bytes = encode(&frames)?;
    Ok(WebpOut {
        bytes,
        width,
        height,
        frames: frames.len(),
        preview_png,
    })
}

fn preview(img: &RgbaImage) -> Result<Vec<u8>, PrepareError> {
    let thumb = DynamicImage::ImageRgba8(img.clone()).thumbnail(PREVIEW_SIDE, PREVIEW_SIDE);
    let mut out = Cursor::new(Vec::new());
    thumb
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|_| PrepareError::Encode)?;
    Ok(out.into_inner())
}

/// Frames (all one size) as WebP; one frame is a still. As the server encodes emoji.
fn encode(frames: &[(RgbaImage, i32)]) -> Result<Vec<u8>, PrepareError> {
    let first = &frames.first().ok_or(PrepareError::Encode)?.0;
    let options = webp_animation::EncoderOptions {
        encoding_config: Some(webp_animation::EncodingConfig::new_lossy(QUALITY)),
        ..Default::default()
    };
    let mut enc = webp_animation::Encoder::new_with_options(first.dimensions(), options)
        .map_err(|_| PrepareError::Encode)?;
    let mut t = 0;
    for (img, ms) in frames {
        enc.add_frame(img.as_raw(), t)
            .map_err(|_| PrepareError::Encode)?;
        t += ms;
    }
    let end = if frames.len() == 1 { 100 } else { t };
    Ok(enc
        .finalize(end)
        .map_err(|_| PrepareError::Encode)?
        .to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame, Rgba};

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = RgbaImage::from_fn(w, h, |x, y| {
            Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255])
        });
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn is_webp(b: &[u8]) -> bool {
        b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP"
    }

    fn decoded(b: &[u8]) -> (u32, u32, usize) {
        let d = WebPDecoder::new(Cursor::new(b)).unwrap();
        let (w, h) = d.dimensions();
        let frames = if d.has_animation() {
            d.into_frames().count()
        } else {
            1
        };
        (w, h, frames)
    }

    #[test]
    fn a_large_png_becomes_webp_capped_at_4096() {
        let out = to_webp(&png(5000, 2500)).unwrap();
        assert!(is_webp(&out.bytes));
        assert_eq!((out.width, out.height, out.frames), (4096, 2048, 1));
        assert_eq!(decoded(&out.bytes), (4096, 2048, 1));
        // the chip's preview is a small PNG
        assert_eq!(
            image::guess_format(&out.preview_png).unwrap(),
            ImageFormat::Png
        );
        let p = image::load_from_memory(&out.preview_png).unwrap();
        assert_eq!((p.width(), p.height()), (96, 48));
    }

    #[test]
    fn a_small_jpeg_keeps_its_size() {
        let img =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(300, 200, Rgba([10, 200, 30, 255])));
        let mut jpeg = Cursor::new(Vec::new());
        img.to_rgb8()
            .write_to(&mut jpeg, ImageFormat::Jpeg)
            .unwrap();
        let out = to_webp(jpeg.get_ref()).unwrap();
        assert!(is_webp(&out.bytes));
        assert_eq!((out.width, out.height), (300, 200));
    }

    #[test]
    fn a_tall_image_is_capped_on_its_long_side() {
        assert_eq!(fit(1000, 9000), (455, 4096));
        assert_eq!(fit(4096, 10), (4096, 10));
        assert_eq!(fit(100_000, 1), (4096, 1));
    }

    #[test]
    fn an_animated_gif_stays_animated() {
        let mut gif = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut gif);
            let frames = (0..3u8).map(|i| {
                Frame::from_parts(
                    RgbaImage::from_pixel(40, 30, Rgba([i * 80, 0, 0, 255])),
                    0,
                    0,
                    Delay::from_numer_denom_ms(100, 1),
                )
            });
            enc.encode_frames(frames).unwrap();
        }
        let out = to_webp(&gif).unwrap();
        assert!(is_webp(&out.bytes));
        assert_eq!(out.frames, 3);
        assert_eq!(decoded(&out.bytes), (40, 30, 3));
    }

    #[test]
    fn a_broken_image_is_not_an_image() {
        let mut bytes = png(50, 50);
        bytes.truncate(60);
        assert_eq!(to_webp(&bytes).err(), Some(PrepareError::NotAnImage));
        assert_eq!(to_webp(b"just text").err(), Some(PrepareError::NotAnImage));
    }

    #[test]
    fn content_types_by_extension() {
        use AttachmentKind::*;
        assert_eq!(content_type_for("clip.MOV"), ("video/quicktime", Video));
        assert_eq!(content_type_for("a.mp4"), ("video/mp4", Video));
        assert_eq!(content_type_for("song.mp3"), ("audio/mpeg", Audio));
        assert_eq!(content_type_for("voice.opus"), ("audio/ogg", Audio));
        assert_eq!(
            content_type_for("a.exe"),
            ("application/octet-stream", File)
        );
        assert_eq!(
            content_type_for("noext"),
            ("application/octet-stream", File)
        );
        // an image that didn't convert goes as a file
        assert_eq!(
            content_type_for("x.png"),
            ("application/octet-stream", File)
        );
    }

    #[test]
    fn names() {
        assert!(is_image_name("Shot.PNG") && is_image_name("a.jpeg") && !is_image_name("a.pdf"));
        assert_eq!(webp_name("photo.PNG"), "photo.webp");
        assert_eq!(webp_name("a.b.gif"), "a.b.webp");
        assert_eq!(webp_name(".png"), ".png.webp");
    }
}
