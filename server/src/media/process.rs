//! Upload processing: sniff, bound, fit to a transparent square and re-encode as WebP. The
//! original is never kept, so its metadata goes with it. Go Live previews go through the same
//! decode bounds and encoder (see [`preview`]).

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::{self, FilterType};
use image::{
    AnimationDecoder, Frames, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits, RgbaImage,
};

/// Largest upload accepted (before processing).
pub const MAX_INPUT: usize = 2 * 1024 * 1024;
/// Largest processed file.
pub const MAX_OUTPUT: usize = 512 * 1024;
pub const MAX_FRAMES: u32 = 300;
/// Emoji canvas side.
pub const SIZE: u32 = 128;
const MAX_SIDE: u32 = 4096;
const MAX_ALLOC: u64 = 256 * 1024 * 1024;
/// Most pixels an animation may decode to, summed over its frames. Decoders composite every frame
/// onto the full logical screen, so a few KB of 1×1 frames on a 4096² screen would otherwise mean
/// minutes of CPU. 100 Mpx still fits 300 frames at 500×500 (75 Mpx), well beyond real emoji GIFs,
/// while a 4096² screen is refused at its 6th frame.
const MAX_DECODED_PIXELS: u64 = 100_000_000;
const QUALITY: f32 = 80.0;

/// Largest Go Live preview upload accepted.
pub const PREVIEW_MAX_INPUT: usize = 100 * 1024;
/// Box a preview is fitted into (the client sends exactly this).
pub const PREVIEW_W: u32 = 480;
pub const PREVIEW_H: u32 = 270;
/// Largest preview side decoded: 4x the box, far beyond what the client sends, and a 16th of the
/// pixels an emoji may have, since a streamer sends one every 15 s.
const PREVIEW_MAX_SIDE: u32 = 1920;

#[derive(Debug)]
pub struct Processed {
    pub webp: Vec<u8>,
    /// First frame alone, for animations only.
    pub still: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("that file is too big (2 MB at most)")]
    TooBig,
    #[error("that isn't a supported image: use PNG, JPEG, GIF or WebP")]
    UnsupportedType,
    #[error("that image couldn't be read; it may be damaged")]
    Corrupt,
    #[error("that animation has too many frames (300 at most)")]
    TooManyFrames,
    #[error("that animation is too big once converted (512 KB at most); try a shorter one")]
    OutputTooBig,
    #[error("that image's dimensions are too large (4096×4096 at most)")]
    TooLarge,
    #[error("this animation is too large to convert; try a smaller or shorter one")]
    AnimationTooLarge,
}

impl From<ImageError> for ProcessError {
    fn from(e: ImageError) -> Self {
        match e {
            ImageError::Limits(_) => Self::TooLarge,
            _ => Self::Corrupt,
        }
    }
}

fn limits() -> Limits {
    limits_with_side(MAX_SIDE)
}

fn limits_with_side(max_side: u32) -> Limits {
    let mut l = Limits::default();
    l.max_image_width = Some(max_side);
    l.max_image_height = Some(max_side);
    l.max_alloc = Some(MAX_ALLOC);
    l
}

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    #[error("the preview is too big (100 KB at most)")]
    TooBig,
    #[error("the preview isn't a usable still image (WebP, PNG or JPEG, 1920×1920 at most)")]
    Unusable,
}

/// Re-encode a Go Live preview frame: sniff (WebP, PNG or JPEG; animations refused), check the
/// header's dimensions before decoding, decode under the media limits, fit within
/// [`PREVIEW_W`]×[`PREVIEW_H`] (aspect kept, never enlarged) and encode a fresh still WebP. Only
/// pixels survive, so the upload's metadata and any extra chunks are gone. CPU-bound: call it off
/// the async runtime.
pub fn preview(input: &[u8]) -> Result<Vec<u8>, PreviewError> {
    if input.len() > PREVIEW_MAX_INPUT {
        return Err(PreviewError::TooBig);
    }
    let format = image::guess_format(input).map_err(|_| PreviewError::Unusable)?;
    // The client asks its webview for WebP; PNG or JPEG are what a webview gives instead when it
    // can't encode WebP.
    if !matches!(
        format,
        ImageFormat::WebP | ImageFormat::Png | ImageFormat::Jpeg
    ) {
        return Err(PreviewError::Unusable);
    }
    let unusable = |_| PreviewError::Unusable;
    let mut reader = ImageReader::with_format(Cursor::new(input), format);
    reader.limits(limits_with_side(PREVIEW_MAX_SIDE));
    reader.into_dimensions().map_err(unusable)?;
    if format == ImageFormat::WebP {
        let d = WebPDecoder::new(Cursor::new(input)).map_err(unusable)?;
        if d.has_animation() {
            return Err(PreviewError::Unusable);
        }
    }
    let mut reader = ImageReader::with_format(Cursor::new(input), format);
    reader.limits(limits_with_side(PREVIEW_MAX_SIDE));
    let img = reader.decode().map_err(unusable)?.into_rgba8();
    let (w, h) = img.dimensions();
    let scale = (PREVIEW_W as f64 / w as f64)
        .min(PREVIEW_H as f64 / h as f64)
        .min(1.0);
    let nw = ((w as f64 * scale).round() as u32).clamp(1, PREVIEW_W);
    let nh = ((h as f64 * scale).round() as u32).clamp(1, PREVIEW_H);
    let fitted = if (nw, nh) == (w, h) {
        img
    } else {
        resize(&img, nw, nh)
    };
    encode(&[(fitted, 0)]).map_err(|_| PreviewError::Unusable)
}

/// Process an emoji upload. CPU-bound: call it off the async runtime.
pub fn emoji(input: &[u8]) -> Result<Processed, ProcessError> {
    emoji_with_budget(input, MAX_DECODED_PIXELS)
}

/// [`emoji`] with the animation pixel budget injected, so tests needn't decode 100 Mpx.
fn emoji_with_budget(input: &[u8], max_decoded_pixels: u64) -> Result<Processed, ProcessError> {
    if input.len() > MAX_INPUT {
        return Err(ProcessError::TooBig);
    }
    let format = image::guess_format(input).map_err(|_| ProcessError::UnsupportedType)?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return Err(ProcessError::UnsupportedType);
    }
    // Dimensions come from the header alone, before anything is decoded.
    let mut reader = ImageReader::with_format(Cursor::new(input), format);
    reader.limits(limits());
    reader.into_dimensions()?;

    let frames: Option<Frames> = match format {
        ImageFormat::Gif => {
            let mut d = GifDecoder::new(Cursor::new(input))?;
            d.set_limits(limits())?;
            Some(d.into_frames())
        }
        ImageFormat::WebP => {
            let mut d = WebPDecoder::new(Cursor::new(input))?;
            d.set_limits(limits())?;
            d.has_animation().then(|| d.into_frames())
        }
        _ => None,
    };
    let mut fitted = Vec::new();
    match frames {
        Some(frames) => {
            let mut decoded = 0u64;
            for f in frames {
                if fitted.len() as u32 == MAX_FRAMES {
                    return Err(ProcessError::TooManyFrames);
                }
                let f = f?;
                // Counted per decoded frame, before the costly resize.
                let (w, h) = f.buffer().dimensions();
                if w > MAX_SIDE || h > MAX_SIDE {
                    return Err(ProcessError::TooLarge);
                }
                decoded += u64::from(w) * u64::from(h);
                if decoded > max_decoded_pixels {
                    return Err(ProcessError::AnimationTooLarge);
                }
                let (n, d) = f.delay().numer_denom_ms();
                fitted.push((fit(f.buffer()), frame_ms(n, d)));
            }
        }
        None => {
            let mut reader = ImageReader::with_format(Cursor::new(input), format);
            reader.limits(limits());
            fitted.push((fit(&reader.decode()?.into_rgba8()), 0));
        }
    }
    if fitted.is_empty() {
        return Err(ProcessError::Corrupt);
    }

    let frames = fitted.len() as u32;
    let webp = encode(&fitted)?;
    let still = (frames > 1).then(|| encode(&fitted[..1])).transpose()?;
    if webp.len() > MAX_OUTPUT {
        return Err(ProcessError::OutputTooBig);
    }
    Ok(Processed {
        webp,
        still,
        width: SIZE,
        height: SIZE,
        frames,
    })
}

/// Browsers show GIF delays of 10 ms or less as 100 ms; keep that, so a GIF looks the same after
/// conversion (and every frame has a duration, which the encoder needs).
fn frame_ms(numer: u32, denom: u32) -> i32 {
    let ms = numer / denom.max(1);
    if ms <= 10 { 100 } else { ms.min(60_000) as i32 }
}

/// Scale to fit `SIZE` (keeping the aspect ratio), centred on a transparent square.
fn fit(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let scale = SIZE as f64 / w.max(h) as f64;
    let nw = ((w as f64 * scale).round() as u32).clamp(1, SIZE);
    let nh = ((h as f64 * scale).round() as u32).clamp(1, SIZE);
    let resized = resize(img, nw, nh);
    let mut canvas = RgbaImage::new(SIZE, SIZE);
    imageops::overlay(
        &mut canvas,
        &resized,
        ((SIZE - nw) / 2).into(),
        ((SIZE - nh) / 2).into(),
    );
    canvas
}

/// Scale `img` to `nw`×`nh`.
fn resize(img: &RgbaImage, nw: u32, nh: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    // Lanczos cost grows with the shrink ratio; a box pre-shrink to twice the target keeps big
    // frames cheap without visible loss.
    if w > 4 * nw || h > 4 * nh {
        let pre = imageops::thumbnail(img, 2 * nw, 2 * nh);
        imageops::resize(&pre, nw, nh, FilterType::Lanczos3)
    } else {
        imageops::resize(img, nw, nh, FilterType::Lanczos3)
    }
}

/// Encode frames (all the same size) as WebP: one frame gives a still image.
fn encode(frames: &[(RgbaImage, i32)]) -> Result<Vec<u8>, ProcessError> {
    let Some((first, _)) = frames.first() else {
        return Err(ProcessError::Corrupt);
    };
    let options = webp_animation::EncoderOptions {
        encoding_config: Some(webp_animation::EncodingConfig::new_lossy(QUALITY)),
        ..Default::default()
    };
    let fail = |e: webp_animation::Error| {
        tracing::error!(error = ?e, "webp encode failed");
        ProcessError::Corrupt
    };
    let mut enc =
        webp_animation::Encoder::new_with_options(first.dimensions(), options).map_err(fail)?;
    let mut t = 0;
    for (img, ms) in frames {
        enc.add_frame(img.as_raw(), t).map_err(fail)?;
        t += ms;
    }
    // A still's single frame gets a nominal duration; it's never shown as an animation.
    let end = if frames.len() == 1 { 100 } else { t };
    Ok(enc.finalize(end).map_err(fail)?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::GifEncoder;
    use image::{AnimationDecoder, Delay, Frame, ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, Rgba([200, 30, 30, 255]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn gif(frames: usize, size: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            for i in 0..frames {
                let shade = (i * 60 % 256) as u8;
                let img = RgbaImage::from_pixel(size, size, Rgba([shade, 0, 255 - shade, 255]));
                let delay = Delay::from_numer_denom_ms(50, 1);
                enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                    .unwrap();
            }
        }
        out
    }

    fn is_webp(b: &[u8]) -> bool {
        b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP"
    }

    #[test]
    fn still_png_becomes_square_webp_within_128() {
        let p = emoji(&png(300, 150)).unwrap();
        assert_eq!((p.width, p.height, p.frames), (128, 128, 1));
        assert!(p.still.is_none());
        assert!(is_webp(&p.webp));
        // Decode what was produced rather than trust the constants.
        let out = image::load_from_memory_with_format(&p.webp, ImageFormat::WebP).unwrap();
        assert_eq!((out.width(), out.height()), (128, 128));
        assert_eq!(image::guess_format(&p.webp).unwrap(), ImageFormat::WebP);
        // The source's colour survives re-encoding (lossy, so within a tolerance). Transparency of
        // the padding isn't asserted: the encoder crops the frame to its content, and how the
        // decoder fills the rest is up to it.
        let centre = out.to_rgba8().get_pixel(64, 64).0;
        for (got, want) in centre.into_iter().zip([200u8, 30, 30, 255]) {
            assert!(got.abs_diff(want) < 12, "{centre:?}");
        }
    }

    #[test]
    fn animated_gif_keeps_frames_and_timing_and_has_a_still() {
        let p = emoji(&gif(3, 40)).unwrap();
        assert_eq!(p.frames, 3);
        let still = p.still.as_deref().unwrap();
        assert!(is_webp(still) && is_webp(&p.webp));
        let dec = image::codecs::webp::WebPDecoder::new(Cursor::new(&p.webp)).unwrap();
        assert!(dec.has_animation());
        let frames = dec.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 3);
        for f in &frames {
            assert_eq!(f.delay().numer_denom_ms(), (50, 1));
            assert_eq!(f.buffer().dimensions(), (128, 128));
        }
    }

    #[test]
    fn single_frame_gif_is_still() {
        let p = emoji(&gif(1, 40)).unwrap();
        assert_eq!(p.frames, 1);
        assert!(p.still.is_none());
    }

    #[test]
    fn extension_is_ignored_real_type_wins() {
        // a PNG "renamed .gif": only the bytes matter
        assert!(emoji(&png(64, 64)).is_ok());
        assert!(matches!(
            emoji(b"hello"),
            Err(ProcessError::UnsupportedType)
        ));
    }

    #[test]
    fn empty_and_oversize_are_refused() {
        assert!(matches!(emoji(&[]), Err(ProcessError::UnsupportedType)));
        let big = vec![0u8; MAX_INPUT + 1];
        assert!(matches!(emoji(&big), Err(ProcessError::TooBig)));
    }

    #[test]
    fn dimension_bomb_is_refused() {
        // GIF89a, logical screen 60000x60000, one 60000x60000 image, trailer. Decoding it would
        // need ~14 GB; the dimensions alone must stop it.
        let mut b = b"GIF89a".to_vec();
        b.extend_from_slice(&60000u16.to_le_bytes());
        b.extend_from_slice(&60000u16.to_le_bytes());
        b.extend_from_slice(&[0, 0, 0]);
        b.push(b',');
        b.extend_from_slice(&[0, 0, 0, 0]);
        b.extend_from_slice(&60000u16.to_le_bytes());
        b.extend_from_slice(&60000u16.to_le_bytes());
        b.extend_from_slice(&[0, 2, 0, b';']);
        assert!(matches!(emoji(&b), Err(ProcessError::TooLarge)));
    }

    #[test]
    fn too_many_frames_is_refused() {
        assert!(matches!(
            emoji(&gif(301, 1)),
            Err(ProcessError::TooManyFrames)
        ));
    }

    /// A tiny GIF with a huge logical screen and many 1x1 frames: each decoded frame is composited
    /// onto the whole screen, so decoding work grows with screen area times frame count.
    fn screen_bomb(screen: u16, frames: usize) -> Vec<u8> {
        let mut b = b"GIF89a".to_vec();
        b.extend_from_slice(&screen.to_le_bytes());
        b.extend_from_slice(&screen.to_le_bytes());
        b.extend_from_slice(&[0x80, 0, 0, 0, 0, 0, 255, 255, 255]);
        for i in 0..frames {
            b.extend_from_slice(&[0x21, 0xF9, 4, 0, 5, 0, 0, 0]);
            b.push(b',');
            b.extend_from_slice(&[(i % 2) as u8, 0, 0, 0, 1, 0, 1, 0, 0]);
            b.extend_from_slice(&[2, 2, 0x4C, 0x01, 0]);
        }
        b.push(b';');
        b
    }

    #[test]
    fn huge_screen_animation_is_refused_quickly() {
        // The budget is injected: a 1024² screen against a 3 Mpx budget trips at its 3rd frame, the
        // same path as 4096² against MAX_DECODED_PIXELS but without seconds of debug-build decoding.
        let bomb = screen_bomb(1024, 300);
        assert!(bomb.len() < 10 * 1024);
        assert!(matches!(
            emoji_with_budget(&bomb, 3_000_000),
            Err(ProcessError::AnimationTooLarge)
        ));
        // The same file under a budget that fits it is not refused for size.
        assert!(emoji_with_budget(&screen_bomb(8, 3), 3_000_000).is_ok());
    }

    /// Frames of differing sizes; each is placed on the first frame's logical screen.
    fn gif_of(sizes: &[(u32, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            for &(w, h) in sizes {
                let img = RgbaImage::from_pixel(w, h, Rgba([9, 9, 9, 255]));
                let delay = Delay::from_numer_denom_ms(50, 1);
                enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                    .unwrap();
            }
        }
        out
    }

    #[test]
    fn a_frame_wider_than_the_side_limit_is_refused() {
        // Small screen, so the header check passes; the second frame alone is 5000 wide.
        assert!(matches!(
            emoji(&gif_of(&[(8, 8), (5000, 1)])),
            Err(ProcessError::TooLarge)
        ));
        assert!(emoji(&gif_of(&[(8, 8), (4096, 1)])).is_ok());
    }

    fn webp_lossless(w: u32, h: u32) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, Rgba([1, 2, 3, 255]));
        let mut out = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut out)
            .encode(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    fn dims(webp: &[u8]) -> (u32, u32) {
        assert!(is_webp(webp));
        let i = image::load_from_memory_with_format(webp, ImageFormat::WebP).unwrap();
        (i.width(), i.height())
    }

    #[test]
    fn preview_side_limit_is_inclusive() {
        assert_eq!(dims(&preview(&webp_lossless(1920, 1)).unwrap()), (480, 1));
        assert!(matches!(
            preview(&webp_lossless(1921, 1)),
            Err(PreviewError::Unusable)
        ));
        assert!(matches!(
            preview(&webp_lossless(1, 1921)),
            Err(PreviewError::Unusable)
        ));
    }

    #[test]
    fn preview_accepts_png_and_jpeg_and_refuses_gif() {
        assert_eq!(dims(&preview(&png(960, 540)).unwrap()), (480, 270));
        let mut jpg = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(RgbaImage::from_pixel(64, 36, Rgba([5, 5, 5, 255])))
            .to_rgb8()
            .write_to(&mut jpg, ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(dims(&preview(jpg.get_ref()).unwrap()), (64, 36));
        assert!(matches!(preview(&gif(1, 8)), Err(PreviewError::Unusable)));
    }

    #[test]
    fn preview_over_the_input_cap_is_refused_unread() {
        let mut big = webp_lossless(8, 8);
        big.resize(PREVIEW_MAX_INPUT + 1, 0);
        assert!(matches!(preview(&big), Err(PreviewError::TooBig)));
    }

    #[test]
    fn errors_read_well() {
        assert!(
            ProcessError::UnsupportedType
                .to_string()
                .contains("PNG, JPEG, GIF or WebP")
        );
    }
}
