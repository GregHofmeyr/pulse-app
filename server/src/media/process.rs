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

/// Quote image: a still is fitted within this on its long side (never enlarged)...
pub const QUOTE_MAX_SIDE: u32 = 1600;
pub const QUOTE_MAX_OUTPUT: usize = 1024 * 1024;
/// ...and an animation within this, since every frame is kept.
pub const QUOTE_ANIM_MAX_SIDE: u32 = 640;
pub const QUOTE_ANIM_MAX_OUTPUT: usize = 4 * 1024 * 1024;
/// Most pixels an animated quote's fitted frames may total (all are held until encoded): about
/// 128 MB of RGBA per upload being processed.
pub const QUOTE_ANIM_MAX_FITTED_PIXELS: u64 = 32_000_000;

/// Avatar and banner uploads: input bounds (the 4096 px side limit is shared with emoji).
pub const PROFILE_MAX_INPUT: usize = 8 * 1024 * 1024;
pub const PROFILE_MAX_FRAMES: u32 = 200;
/// Avatar: a square, still or animated.
pub const AVATAR_SIDE: u32 = 256;
pub const AVATAR_MAX_OUTPUT: usize = 512 * 1024;
pub const AVATAR_ANIM_MAX_OUTPUT: usize = 1024 * 1024;
/// Banner: 960×384 still; an animation is smaller (600×240), since every frame is held.
pub const BANNER_W: u32 = 960;
pub const BANNER_H: u32 = 384;
pub const BANNER_ANIM_W: u32 = 600;
pub const BANNER_ANIM_H: u32 = 240;
pub const BANNER_MAX_OUTPUT: usize = 1024 * 1024;
pub const BANNER_ANIM_MAX_OUTPUT: usize = 2 * 1024 * 1024;
/// Most pixels an animated avatar's or banner's fitted frames may total (about 128 MB of RGBA).
pub const PROFILE_ANIM_MAX_FITTED_PIXELS: u64 = 32_000_000;

/// Largest Go Live preview upload accepted.
pub const PREVIEW_MAX_INPUT: usize = 100 * 1024;
/// Box a preview is fitted into (the client sends exactly this).
pub const PREVIEW_W: u32 = 480;
pub const PREVIEW_H: u32 = 270;
/// Largest preview side decoded: 4x the box and far beyond what the client sends. That is under a
/// quarter of the pixels an emoji may have, since a streamer sends one every 15 s.
const PREVIEW_MAX_SIDE: u32 = 1920;

/// A rectangle in source pixels, applied to every frame before fitting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

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
    #[error("that image is too big once converted; try a smaller or shorter one")]
    QuoteOutputTooBig,
    #[error("that image's dimensions are too large (4096×4096 at most)")]
    TooLarge,
    #[error("this animation is too large to convert; try a smaller or shorter one")]
    AnimationTooLarge,
    #[error("that file is too big (8 MB at most)")]
    ProfileTooBig,
    #[error("that animation has too many frames (200 at most)")]
    ProfileTooManyFrames,
    #[error("that animation is too big once converted (1 MB at most); try a shorter one")]
    AvatarOutputTooBig,
    #[error("that animation is too big once converted (2 MB at most); try a shorter one")]
    BannerOutputTooBig,
    #[error("The crop is outside the image.")]
    CropOutside,
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
    #[error(
        "the preview isn't a usable still image of its declared type (WebP, PNG or JPEG, 1920×1920 at most)"
    )]
    Unusable,
    /// Encoding failed: the server's fault, not the upload's.
    #[error("the preview couldn't be encoded")]
    Encode,
}

/// The image format a preview's `Content-Type` declares, if it is one previews may be.
pub fn preview_format(content_type: &str) -> Option<ImageFormat> {
    match content_type.trim().to_ascii_lowercase().as_str() {
        "image/webp" => Some(ImageFormat::WebP),
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        _ => None,
    }
}

/// Re-encode a Go Live preview frame: sniff (it must be the `declared` format, one of WebP, PNG or
/// JPEG; animations refused), check the
/// header's dimensions before decoding, decode under the media limits, fit within
/// [`PREVIEW_W`]×[`PREVIEW_H`] (aspect kept, never enlarged) and encode a fresh still WebP. Only
/// pixels survive, so the upload's metadata and any extra chunks are gone. CPU-bound: call it off
/// the async runtime.
pub fn preview(input: &[u8], declared: ImageFormat) -> Result<Vec<u8>, PreviewError> {
    if input.len() > PREVIEW_MAX_INPUT {
        return Err(PreviewError::TooBig);
    }
    let format = image::guess_format(input).map_err(|_| PreviewError::Unusable)?;
    // The client asks its webview for WebP, and falls back to JPEG when the webview can't encode
    // WebP; PNG is that fallback's own fallback.
    if format != declared
        || !matches!(
            format,
            ImageFormat::WebP | ImageFormat::Png | ImageFormat::Jpeg
        )
    {
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
    encode(&[(fitted, 0)]).map_err(|_| PreviewError::Encode)
}

/// How an upload is fitted and bounded: the per-frame `fit` (given the box to fit to), the largest
/// input and frame count, and, for a still and for an animation, the box and the largest processed
/// file.
struct Profile {
    fit: fn(&RgbaImage, (u32, u32)) -> RgbaImage,
    max_input: usize,
    max_frames: u32,
    still_box: (u32, u32),
    still_max: usize,
    anim_box: (u32, u32),
    anim_max: usize,
    /// Most pixels the fitted frames of an animation may total.
    anim_max_pixels: u64,
}

/// Emoji: a transparent square, still or animated.
const EMOJI: Profile = Profile {
    fit: |img, _| fit(img),
    max_input: MAX_INPUT,
    max_frames: MAX_FRAMES,
    still_box: (SIZE, SIZE),
    still_max: MAX_OUTPUT,
    anim_box: (SIZE, SIZE),
    anim_max: MAX_OUTPUT,
    // 300 frames of 128², already its natural bound.
    anim_max_pixels: u64::MAX,
};

/// Quote images: aspect kept, never enlarged, fitted on the long side to the box's width.
const QUOTE: Profile = Profile {
    fit: |img, b| fit_long_side(img, b.0),
    max_input: MAX_INPUT,
    max_frames: MAX_FRAMES,
    still_box: (QUOTE_MAX_SIDE, QUOTE_MAX_SIDE),
    still_max: QUOTE_MAX_OUTPUT,
    anim_box: (QUOTE_ANIM_MAX_SIDE, QUOTE_ANIM_MAX_SIDE),
    anim_max: QUOTE_ANIM_MAX_OUTPUT,
    anim_max_pixels: QUOTE_ANIM_MAX_FITTED_PIXELS,
};

/// Avatars: covered to a square.
const AVATAR: Profile = Profile {
    fit: fit_cover,
    max_input: PROFILE_MAX_INPUT,
    max_frames: PROFILE_MAX_FRAMES,
    still_box: (AVATAR_SIDE, AVATAR_SIDE),
    still_max: AVATAR_MAX_OUTPUT,
    anim_box: (AVATAR_SIDE, AVATAR_SIDE),
    anim_max: AVATAR_ANIM_MAX_OUTPUT,
    anim_max_pixels: PROFILE_ANIM_MAX_FITTED_PIXELS,
};

/// Banners: covered to a wide box, smaller when animated.
const BANNER: Profile = Profile {
    fit: fit_cover,
    max_input: PROFILE_MAX_INPUT,
    max_frames: PROFILE_MAX_FRAMES,
    still_box: (BANNER_W, BANNER_H),
    still_max: BANNER_MAX_OUTPUT,
    anim_box: (BANNER_ANIM_W, BANNER_ANIM_H),
    anim_max: BANNER_ANIM_MAX_OUTPUT,
    anim_max_pixels: PROFILE_ANIM_MAX_FITTED_PIXELS,
};

/// Process an emoji upload. CPU-bound: call it off the async runtime.
pub fn emoji(input: &[u8]) -> Result<Processed, ProcessError> {
    emoji_with_budget(input, MAX_DECODED_PIXELS)
}

/// Process a quote image upload (same input rules as [`emoji`]). CPU-bound: call it off the async
/// runtime.
pub fn quote_image(input: &[u8]) -> Result<Processed, ProcessError> {
    quote_with(input, &QUOTE)
}

/// [`quote_image`] with the profile injected, so tests can use small caps.
fn quote_with(input: &[u8], profile: &Profile) -> Result<Processed, ProcessError> {
    convert(input, None, MAX_DECODED_PIXELS, profile).map_err(|e| match e {
        ProcessError::OutputTooBig => ProcessError::QuoteOutputTooBig,
        e => e,
    })
}

/// Process an avatar upload: an optional crop on every frame, then covered to 256×256. CPU-bound:
/// call it off the async runtime.
pub fn avatar(input: &[u8], crop: Option<Crop>) -> Result<Processed, ProcessError> {
    avatar_with(input, crop, &AVATAR)
}

/// Process a banner upload: an optional crop on every frame, then covered to 960×384 (600×240
/// when animated). CPU-bound: call it off the async runtime.
pub fn banner(input: &[u8], crop: Option<Crop>) -> Result<Processed, ProcessError> {
    banner_with(input, crop, &BANNER)
}

fn avatar_with(
    input: &[u8],
    crop: Option<Crop>,
    profile: &Profile,
) -> Result<Processed, ProcessError> {
    profile_with(input, crop, profile, ProcessError::AvatarOutputTooBig)
}

fn banner_with(
    input: &[u8],
    crop: Option<Crop>,
    profile: &Profile,
) -> Result<Processed, ProcessError> {
    profile_with(input, crop, profile, ProcessError::BannerOutputTooBig)
}

/// [`convert`] for a profile picture, with the errors that name its limits.
fn profile_with(
    input: &[u8],
    crop: Option<Crop>,
    profile: &Profile,
    output_too_big: ProcessError,
) -> Result<Processed, ProcessError> {
    convert(input, crop, MAX_DECODED_PIXELS, profile).map_err(|e| match e {
        ProcessError::TooBig => ProcessError::ProfileTooBig,
        ProcessError::TooManyFrames => ProcessError::ProfileTooManyFrames,
        ProcessError::OutputTooBig => output_too_big,
        e => e,
    })
}

/// [`emoji`] with the animation pixel budget injected, so tests needn't decode 100 Mpx.
fn emoji_with_budget(input: &[u8], max_decoded_pixels: u64) -> Result<Processed, ProcessError> {
    convert(input, None, max_decoded_pixels, &EMOJI)
}

/// The pipeline every kind shares: sniff, bound, decode, crop, fit, re-encode.
fn convert(
    input: &[u8],
    crop: Option<Crop>,
    max_decoded_pixels: u64,
    profile: &Profile,
) -> Result<Processed, ProcessError> {
    if input.len() > profile.max_input {
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
    let (logical_w, logical_h) = reader.into_dimensions()?;
    if let Some(c) = crop {
        let inside = c.w >= 1
            && c.h >= 1
            && u64::from(c.x) + u64::from(c.w) <= u64::from(logical_w)
            && u64::from(c.y) + u64::from(c.h) <= u64::from(logical_h);
        if !inside {
            return Err(ProcessError::CropOutside);
        }
    }

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
            let mut n = 0u32;
            let mut fitted_pixels = 0u64;
            // The first frame waits for the second: only then is it known to be an animation,
            // which is fitted differently from a still.
            let mut first: Option<(RgbaImage, i32)> = None;
            for f in frames {
                if n == profile.max_frames {
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
                let (num, den) = f.delay().numer_denom_ms();
                let ms = frame_ms(num, den);
                n += 1;
                if n == 1 {
                    first = Some((cropped(f.into_buffer(), crop), ms));
                    continue;
                }
                // All fitted frames are held until encoding, so their total is bounded too.
                let mut push = |img: RgbaImage, ms| {
                    fitted_pixels += u64::from(img.width()) * u64::from(img.height());
                    fitted.push((img, ms));
                    if fitted_pixels > profile.anim_max_pixels {
                        return Err(ProcessError::AnimationTooLarge);
                    }
                    Ok(())
                };
                if let Some((img, ms)) = first.take() {
                    push((profile.fit)(&img, profile.anim_box), ms)?;
                }
                push(
                    (profile.fit)(&cropped(f.into_buffer(), crop), profile.anim_box),
                    ms,
                )?;
            }
            if let Some((img, ms)) = first {
                fitted.push(((profile.fit)(&img, profile.still_box), ms));
            }
        }
        None => {
            let mut reader = ImageReader::with_format(Cursor::new(input), format);
            reader.limits(limits());
            let img = cropped(reader.decode()?.into_rgba8(), crop);
            fitted.push(((profile.fit)(&img, profile.still_box), 0));
        }
    }
    if fitted.is_empty() {
        return Err(ProcessError::Corrupt);
    }

    let frames = fitted.len() as u32;
    let webp = encode(&fitted)?;
    let still = (frames > 1).then(|| encode(&fitted[..1])).transpose()?;
    let max_output = if frames > 1 {
        profile.anim_max
    } else {
        profile.still_max
    };
    if webp.len() > max_output {
        return Err(ProcessError::OutputTooBig);
    }
    let (width, height) = fitted[0].0.dimensions();
    Ok(Processed {
        webp,
        still,
        width,
        height,
        frames,
    })
}

/// Browsers show GIF delays of 10 ms or less as 100 ms; keep that, so a GIF looks the same after
/// conversion (and every frame has a duration, which the encoder needs).
fn frame_ms(numer: u32, denom: u32) -> i32 {
    let ms = numer / denom.max(1);
    if ms <= 10 { 100 } else { ms.min(60_000) as i32 }
}

/// `img` cut to `crop` (already checked to lie inside the image), if there is one.
fn cropped(img: RgbaImage, crop: Option<Crop>) -> RgbaImage {
    match crop {
        Some(c) => imageops::crop_imm(&img, c.x, c.y, c.w, c.h).to_image(),
        None => img,
    }
}

/// Centre-crop to the aspect of `(w, h)`, then resize to exactly `w`×`h` (small sources are
/// enlarged).
fn fit_cover(img: &RgbaImage, (w, h): (u32, u32)) -> RgbaImage {
    let (iw, ih) = img.dimensions();
    // Compare iw/ih with w/h by cross-multiplying.
    let (cw, ch) = if u64::from(iw) * u64::from(h) > u64::from(ih) * u64::from(w) {
        // Wider than the box: trim the sides.
        (
            ((u64::from(ih) * u64::from(w)) / u64::from(h)).max(1) as u32,
            ih,
        )
    } else {
        (
            iw,
            ((u64::from(iw) * u64::from(h)) / u64::from(w)).max(1) as u32,
        )
    };
    let (cw, ch) = (cw.min(iw), ch.min(ih));
    let view = imageops::crop_imm(img, (iw - cw) / 2, (ih - ch) / 2, cw, ch).to_image();
    resize(&view, w, h)
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

/// Scale to fit `side` on the long side, keeping the aspect ratio and never enlarging.
fn fit_long_side(img: &RgbaImage, side: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    if w.max(h) <= side {
        return img.clone();
    }
    let scale = side as f64 / w.max(h) as f64;
    let nw = ((w as f64 * scale).round() as u32).clamp(1, side);
    let nh = ((h as f64 * scale).round() as u32).clamp(1, side);
    resize(img, nw, nh)
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

    fn gif_sized(frames: usize, w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            for i in 0..frames {
                let shade = (i * 60 % 256) as u8;
                let img = RgbaImage::from_pixel(w, h, Rgba([shade, 0, 255 - shade, 255]));
                let delay = Delay::from_numer_denom_ms(50, 1);
                enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                    .unwrap();
            }
        }
        out
    }

    #[test]
    fn quote_image_fits_long_side_and_never_enlarges() {
        let p = quote_image(&png(3000, 1000)).unwrap();
        assert_eq!((p.width, p.height, p.frames), (1600, 533, 1));
        assert!(p.still.is_none());
        assert_eq!(dims(&p.webp), (1600, 533));
        let p = quote_image(&png(200, 100)).unwrap();
        assert_eq!((p.width, p.height), (200, 100));
        assert_eq!(dims(&p.webp), (200, 100));
    }

    #[test]
    fn quote_image_keeps_an_animation_with_a_still() {
        let p = quote_image(&gif_sized(3, 1000, 500)).unwrap();
        assert_eq!((p.frames, p.width, p.height), (3, 640, 320));
        let dec = WebPDecoder::new(Cursor::new(&p.webp)).unwrap();
        assert!(dec.has_animation());
        let frames = dec.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 3);
        for f in &frames {
            assert_eq!(f.delay().numer_denom_ms(), (50, 1));
            assert_eq!(f.buffer().dimensions(), (640, 320));
        }
        let still = p.still.as_deref().unwrap();
        assert_eq!(dims(still), (640, 320));
        let dec = WebPDecoder::new(Cursor::new(still)).unwrap();
        assert!(!dec.has_animation());
        // The still is the first frame: its colour is frame 1's (0,0,255), not frame 2's (60,0,195).
        let px = image::load_from_memory_with_format(still, ImageFormat::WebP)
            .unwrap()
            .to_rgba8()
            .get_pixel(320, 160)
            .0;
        assert!(px[0].abs_diff(0) < 20 && px[2].abs_diff(255) < 20, "{px:?}");
        assert!(px[2].abs_diff(195) > 40, "{px:?}");
    }

    /// Incompressible pixels, so the encoded size is predictable enough for a tiny cap.
    fn noise(w: u32, h: u32, seed: u32) -> RgbaImage {
        let mut x = seed.wrapping_mul(2654435761).wrapping_add(1);
        RgbaImage::from_fn(w, h, |_, _| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            Rgba([x as u8, (x >> 8) as u8, (x >> 16) as u8, 255])
        })
    }

    fn noisy_gif(frames: u32, size: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            for i in 0..frames {
                let delay = Delay::from_numer_denom_ms(50, 1);
                enc.encode_frame(Frame::from_parts(noise(size, size, i), 0, 0, delay))
                    .unwrap();
            }
        }
        out
    }

    fn noisy_png(size: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        noise(size, size, 7)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn quote_output_caps_depend_on_still_or_animation() {
        let tiny = 2_000;
        // A still obeys the still cap (not the animation one), and says so in quote terms.
        let still_capped = Profile {
            still_max: tiny,
            ..QUOTE
        };
        assert!(matches!(
            quote_with(&noisy_png(64), &still_capped),
            Err(ProcessError::QuoteOutputTooBig)
        ));
        let anim_capped = Profile {
            anim_max: tiny,
            ..QUOTE
        };
        assert!(quote_with(&noisy_png(64), &anim_capped).is_ok());
        // An animation obeys the animation cap (not the still one).
        assert!(matches!(
            quote_with(&noisy_gif(3, 64), &anim_capped),
            Err(ProcessError::QuoteOutputTooBig)
        ));
        assert!(quote_with(&noisy_gif(3, 64), &still_capped).is_ok());
        // Emoji keep their own error.
        assert!(matches!(
            convert(
                &noisy_png(64),
                None,
                MAX_DECODED_PIXELS,
                &Profile {
                    still_max: tiny,
                    ..EMOJI
                }
            ),
            Err(ProcessError::OutputTooBig)
        ));
    }

    #[test]
    fn quote_animation_over_the_fitted_pixel_budget_is_refused() {
        // 3 frames of 64² = 12 288 fitted pixels.
        let gif = gif_sized(3, 64, 64);
        let at = |max| Profile {
            anim_max_pixels: max,
            ..QUOTE
        };
        assert!(quote_with(&gif, &at(12_288)).is_ok());
        assert!(matches!(
            quote_with(&gif, &at(12_287)),
            Err(ProcessError::AnimationTooLarge)
        ));
        // A still isn't an animation: the budget doesn't apply to it.
        assert!(quote_with(&png(64, 64), &at(1)).is_ok());
        assert!(quote_with(&gif_sized(1, 64, 64), &at(1)).is_ok());
    }

    #[test]
    fn a_single_frame_gif_quote_is_still() {
        let p = quote_image(&gif_sized(1, 1000, 500)).unwrap();
        assert_eq!((p.frames, p.width, p.height), (1, 1000, 500));
        assert!(p.still.is_none());
    }

    #[test]
    fn quote_image_refuses_what_emoji_refuse() {
        assert!(matches!(
            quote_image(&gif_of(&[(5000, 1)])),
            Err(ProcessError::TooLarge)
        ));
        assert!(matches!(
            quote_image(b"hello"),
            Err(ProcessError::UnsupportedType)
        ));
        assert!(matches!(
            quote_image(&vec![0u8; MAX_INPUT + 1]),
            Err(ProcessError::TooBig)
        ));
        assert!(matches!(
            quote_image(&gif(301, 1)),
            Err(ProcessError::TooManyFrames)
        ));
    }

    #[test]
    fn preview_side_limit_is_inclusive() {
        assert_eq!(
            dims(&preview(&webp_lossless(1920, 1), ImageFormat::WebP).unwrap()),
            (480, 1)
        );
        assert!(matches!(
            preview(&webp_lossless(1921, 1), ImageFormat::WebP),
            Err(PreviewError::Unusable)
        ));
        assert!(matches!(
            preview(&webp_lossless(1, 1921), ImageFormat::WebP),
            Err(PreviewError::Unusable)
        ));
    }

    #[test]
    fn preview_accepts_png_and_jpeg_and_refuses_gif() {
        assert_eq!(
            dims(&preview(&png(960, 540), ImageFormat::Png).unwrap()),
            (480, 270)
        );
        let mut jpg = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(RgbaImage::from_pixel(64, 36, Rgba([5, 5, 5, 255])))
            .to_rgb8()
            .write_to(&mut jpg, ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(
            dims(&preview(jpg.get_ref(), ImageFormat::Jpeg).unwrap()),
            (64, 36)
        );
        assert!(matches!(
            preview(&gif(1, 8), ImageFormat::Gif),
            Err(PreviewError::Unusable)
        ));
    }

    #[test]
    fn preview_types_are_webp_png_and_jpeg_only() {
        assert_eq!(preview_format("image/webp"), Some(ImageFormat::WebP));
        assert_eq!(preview_format(" Image/PNG "), Some(ImageFormat::Png));
        assert_eq!(preview_format("image/jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(preview_format("image/gif"), None);
        assert_eq!(preview_format("image/svg+xml"), None);
        // and the bytes must agree with it
        assert!(matches!(
            preview(&png(8, 8), ImageFormat::WebP),
            Err(PreviewError::Unusable)
        ));
    }

    #[test]
    fn preview_over_the_input_cap_is_refused_unread() {
        let mut big = webp_lossless(8, 8);
        big.resize(PREVIEW_MAX_INPUT + 1, 0);
        assert!(matches!(
            preview(&big, ImageFormat::WebP),
            Err(PreviewError::TooBig)
        ));
    }

    #[test]
    fn errors_read_well() {
        assert!(
            ProcessError::UnsupportedType
                .to_string()
                .contains("PNG, JPEG, GIF or WebP")
        );
        for (e, msg) in [
            (
                ProcessError::ProfileTooBig,
                "that file is too big (8 MB at most)",
            ),
            (
                ProcessError::ProfileTooManyFrames,
                "that animation has too many frames (200 at most)",
            ),
            (
                ProcessError::AvatarOutputTooBig,
                "that animation is too big once converted (1 MB at most); try a shorter one",
            ),
            (
                ProcessError::BannerOutputTooBig,
                "that animation is too big once converted (2 MB at most); try a shorter one",
            ),
            (ProcessError::CropOutside, "The crop is outside the image."),
        ] {
            assert_eq!(e.to_string(), msg);
        }
    }

    /// A GIF of `frames` frames, `w`×`h`, the left half red and the right half blue.
    fn split_gif(frames: usize, w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            for i in 0..frames {
                // The blue half differs a little per frame, or the encoder merges the frames.
                let img = RgbaImage::from_fn(w, h, |x, _| {
                    if x < w / 2 {
                        Rgba([255, 0, 0, 255])
                    } else {
                        Rgba([0, 0, 255 - 15 * i as u8, 255])
                    }
                });
                let delay = Delay::from_numer_denom_ms(50, 1);
                enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                    .unwrap();
            }
        }
        out
    }

    fn centre(webp: &[u8]) -> [u8; 4] {
        let img = image::load_from_memory_with_format(webp, ImageFormat::WebP)
            .unwrap()
            .to_rgba8();
        img.get_pixel(img.width() / 2, img.height() / 2).0
    }

    #[test]
    fn avatar_is_a_256_square() {
        let p = avatar(&png(300, 150), None).unwrap();
        assert_eq!((p.width, p.height, p.frames), (256, 256, 1));
        assert!(p.still.is_none());
        assert_eq!(dims(&p.webp), (256, 256));
    }

    #[test]
    fn avatar_crop_is_applied_to_every_frame() {
        let crop = Crop {
            x: 100,
            y: 0,
            w: 100,
            h: 100,
        };
        let p = avatar(&split_gif(3, 200, 100), Some(crop)).unwrap();
        assert_eq!((p.frames, p.width, p.height), (3, 256, 256));
        let dec = WebPDecoder::new(Cursor::new(&p.webp)).unwrap();
        let frames = dec.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 3);
        for f in &frames {
            assert_eq!(f.buffer().dimensions(), (256, 256));
            let px = f.buffer().get_pixel(128, 128).0;
            assert!(
                px[0] < 40 && px[2] > 190,
                "centre should be blue, got {px:?}"
            );
        }
        let still = p.still.as_deref().unwrap();
        assert_eq!(dims(still), (256, 256));
        assert!(
            !WebPDecoder::new(Cursor::new(still))
                .unwrap()
                .has_animation()
        );
        let px = centre(still);
        assert!(px[0] < 40 && px[2] > 190, "{px:?}");
    }

    #[test]
    fn banner_still_and_animated_sizes() {
        let p = banner(&png(1920, 768), None).unwrap();
        assert_eq!((p.width, p.height, p.frames), (960, 384, 1));
        assert!(p.still.is_none());
        let p = banner(&gif_sized(3, 1200, 480), None).unwrap();
        assert_eq!((p.width, p.height, p.frames), (600, 240, 3));
        assert_eq!(dims(p.still.as_deref().unwrap()), (600, 240));
    }

    #[test]
    fn small_sources_are_enlarged_to_the_box() {
        let p = avatar(&png(64, 64), None).unwrap();
        assert_eq!((p.width, p.height), (256, 256));
        assert_eq!(dims(&p.webp), (256, 256));
    }

    #[test]
    fn a_crop_outside_the_image_is_refused() {
        let past = Crop {
            x: 250,
            y: 0,
            w: 100,
            h: 100,
        };
        assert!(matches!(
            avatar(&png(300, 150), Some(past)),
            Err(ProcessError::CropOutside)
        ));
        let empty = Crop {
            x: 0,
            y: 0,
            w: 0,
            h: 10,
        };
        assert!(matches!(
            banner(&png(300, 150), Some(empty)),
            Err(ProcessError::CropOutside)
        ));
        let empty = Crop {
            x: 0,
            y: 0,
            w: 10,
            h: 0,
        };
        assert!(matches!(
            avatar(&png(300, 150), Some(empty)),
            Err(ProcessError::CropOutside)
        ));
        // The exact edge is inside.
        let edge = Crop {
            x: 200,
            y: 50,
            w: 100,
            h: 100,
        };
        assert!(avatar(&png(300, 150), Some(edge)).is_ok());
    }

    #[test]
    fn profile_uploads_refuse_what_they_should() {
        assert!(matches!(
            avatar(&vec![0u8; PROFILE_MAX_INPUT + 1], None),
            Err(ProcessError::ProfileTooBig)
        ));
        assert!(matches!(
            banner(&vec![0u8; PROFILE_MAX_INPUT + 1], None),
            Err(ProcessError::ProfileTooBig)
        ));
        assert!(matches!(
            avatar(&gif(201, 1), None),
            Err(ProcessError::ProfileTooManyFrames)
        ));
        assert!(matches!(
            avatar(&gif_of(&[(5000, 1)]), None),
            Err(ProcessError::TooLarge)
        ));
        assert!(matches!(
            banner(b"hello", None),
            Err(ProcessError::UnsupportedType)
        ));
    }

    #[test]
    fn profile_output_caps() {
        let tiny = 2_000;
        let still_capped = Profile {
            still_max: tiny,
            ..AVATAR
        };
        assert!(matches!(
            avatar_with(&noisy_png(64), None, &still_capped),
            Err(ProcessError::AvatarOutputTooBig)
        ));
        let anim_capped = Profile {
            anim_max: tiny,
            ..AVATAR
        };
        assert!(avatar_with(&noisy_png(64), None, &anim_capped).is_ok());
        assert!(matches!(
            avatar_with(&noisy_gif(3, 64), None, &anim_capped),
            Err(ProcessError::AvatarOutputTooBig)
        ));
        assert!(avatar_with(&noisy_gif(3, 64), None, &still_capped).is_ok());

        let still_capped = Profile {
            still_max: tiny,
            ..BANNER
        };
        assert!(matches!(
            banner_with(&noisy_png(64), None, &still_capped),
            Err(ProcessError::BannerOutputTooBig)
        ));
        let anim_capped = Profile {
            anim_max: tiny,
            ..BANNER
        };
        assert!(banner_with(&noisy_png(64), None, &anim_capped).is_ok());
        assert!(matches!(
            banner_with(&noisy_gif(3, 64), None, &anim_capped),
            Err(ProcessError::BannerOutputTooBig)
        ));
    }
}
