//! Soundboard limits and the Ogg Opus reader. The server validates uploads with it and the
//! client core demuxes with it, so there is one parser and one set of limits.

use std::borrow::Cow;
use std::fmt;

/// Longest sound the server accepts. The client trims to 6 s; the rest is slack for the encoder.
pub const MAX_DURATION_MS: u32 = 6200;
pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_NAME: usize = 32;
const MAX_EMOJI: usize = 16;

/// A parsed Ogg Opus stream: just what playback and validation need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OggOpus<'a> {
    pub channels: u8,
    pub pre_skip: u16,
    /// Audio packets only (the two header packets are left out). A packet that spans pages is
    /// joined, so it is owned; one within a page borrows from the input.
    pub packets: Vec<Cow<'a, [u8]>>,
    pub duration_ms: u32,
}

/// Why a file isn't an acceptable sound. The messages are shown to people.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundFileError {
    NotOgg,
    BadChecksum,
    NotOpus,
    Channels,
    Truncated,
    Empty,
    TooLong,
    TooLarge,
    /// Far more packets than its length could hold: a decode bomb, not a sound.
    TooManyPackets,
}

impl fmt::Display for SoundFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotOgg | Self::NotOpus => "that isn't an Ogg Opus file",
            Self::BadChecksum | Self::Truncated | Self::TooManyPackets => {
                "that sound file is damaged"
            }
            Self::Channels => "sounds must be mono or stereo",
            Self::Empty => "that sound is empty",
            Self::TooLong => "sounds can be at most 6 seconds",
            Self::TooLarge => "that sound file is too big",
        })
    }
}

impl std::error::Error for SoundFileError {}

/// Ogg's CRC-32: polynomial 0x04c11db7, initial value 0, no reflection, no final xor.
fn ogg_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Read an Ogg Opus file: check every page's CRC, find `OpusHead`, collect the audio packets
/// and work out the length from the last page's granule position (always counted at 48 kHz).
pub fn parse_ogg_opus(bytes: &[u8]) -> Result<OggOpus<'_>, SoundFileError> {
    if bytes.len() > MAX_BYTES {
        return Err(SoundFileError::TooLarge);
    }
    let mut packets: Vec<Cow<[u8]>> = Vec::new();
    // The unfinished packet carried over from the previous page.
    let mut carry: Option<Vec<u8>> = None;
    let mut serial = None;
    let mut last_granule = -1i64;
    let mut pos = 0;
    while pos < bytes.len() {
        let rest = &bytes[pos..];
        if rest.len() < 27 || &rest[..4] != b"OggS" || rest[4] != 0 {
            return Err(SoundFileError::NotOgg);
        }
        let nsegs = rest[26] as usize;
        let header_len = 27 + nsegs;
        if rest.len() < header_len {
            return Err(SoundFileError::Truncated);
        }
        let lacing = &rest[27..header_len];
        let body_len: usize = lacing.iter().map(|&l| l as usize).sum();
        let page_len = header_len + body_len;
        if rest.len() < page_len {
            return Err(SoundFileError::Truncated);
        }
        let page = &rest[..page_len];
        let stored = u32::from_le_bytes(page[22..26].try_into().unwrap());
        let mut zeroed = page.to_vec();
        zeroed[22..26].fill(0);
        if ogg_crc(&zeroed) != stored {
            return Err(SoundFileError::BadChecksum);
        }
        let page_serial = u32::from_le_bytes(page[14..18].try_into().unwrap());
        if *serial.get_or_insert(page_serial) != page_serial {
            // Chained or multiplexed streams aren't a single sound.
            return Err(SoundFileError::NotOpus);
        }
        let continued = page[5] & 1 != 0;
        if continued != carry.is_some() {
            return Err(SoundFileError::Truncated);
        }
        last_granule = i64::from_le_bytes(page[6..14].try_into().unwrap());

        let body = &page[header_len..];
        let (mut start, mut at) = (0usize, 0usize);
        for &l in lacing {
            at += l as usize;
            if l == 255 {
                continue;
            }
            let piece = &body[start..at];
            start = at;
            packets.push(match carry.take() {
                Some(mut joined) => {
                    joined.extend_from_slice(piece);
                    Cow::Owned(joined)
                }
                None => Cow::Borrowed(piece),
            });
        }
        if start < at {
            let mut part = carry.take().unwrap_or_default();
            part.extend_from_slice(&body[start..at]);
            carry = Some(part);
        }
        pos += page_len;
    }
    if carry.is_some() || packets.is_empty() {
        return Err(SoundFileError::Truncated);
    }

    let head = &packets[0];
    if head.len() < 19 || &head[..8] != b"OpusHead" || head[8] >> 4 != 0 {
        return Err(SoundFileError::NotOpus);
    }
    let channels = head[9];
    if !(1..=2).contains(&channels) {
        return Err(SoundFileError::Channels);
    }
    let pre_skip = u16::from_le_bytes([head[10], head[11]]);
    if packets.len() < 2 || !packets[1].starts_with(b"OpusTags") {
        return Err(SoundFileError::NotOpus);
    }
    let audio: Vec<Cow<[u8]>> = packets.into_iter().skip(2).collect();
    if audio.is_empty() {
        return Err(SoundFileError::Empty);
    }

    // checked first: a hostile negative granule minus the pre-skip would overflow
    if last_granule < 0 {
        return Err(SoundFileError::Empty);
    }
    let samples = last_granule - i64::from(pre_skip);
    if samples <= 0 {
        return Err(SoundFileError::Empty);
    }
    let ms = samples / 48;
    if ms == 0 {
        return Err(SoundFileError::Empty);
    }
    if ms > i64::from(MAX_DURATION_MS) {
        return Err(SoundFileError::TooLong);
    }
    // Only the last granule is checked above, so 256 KiB of tiny packets could still hold far
    // more audio than it claims (every listener would decode it all). An Opus packet is at least
    // 2.5 ms, so a real file never has more than this.
    if audio.len() as i64 > ms * 2 / 5 + 64 {
        return Err(SoundFileError::TooManyPackets);
    }
    Ok(OggOpus {
        channels,
        pre_skip,
        packets: audio,
        duration_ms: ms as u32,
    })
}

/// A sound's name as it will be stored: trimmed, 1 to [`MAX_NAME`] characters, no control characters.
pub fn clean_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let n = name.chars().count();
    (1..=MAX_NAME)
        .contains(&n)
        .then_some(name)
        .filter(|s| !s.chars().any(char::is_control))
        .map(str::to_owned)
}

/// A sound's emoji: 1 to 16 characters, nothing typed (no ASCII letters or digits), no whitespace
/// or control characters. Unicode emoji only.
pub fn valid_emoji(s: &str) -> bool {
    (1..=MAX_EMOJI).contains(&s.chars().count())
        && !s
            .chars()
            .any(|c| c.is_ascii_alphanumeric() || c.is_whitespace() || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(header_type: u8, granule: i64, seq: u32, lacing: &[u8], body: &[u8]) -> Vec<u8> {
        let mut p = b"OggS".to_vec();
        p.push(0);
        p.push(header_type);
        p.extend_from_slice(&granule.to_le_bytes());
        p.extend_from_slice(&7u32.to_le_bytes());
        p.extend_from_slice(&seq.to_le_bytes());
        p.extend_from_slice(&[0; 4]);
        p.push(lacing.len() as u8);
        p.extend_from_slice(lacing);
        p.extend_from_slice(body);
        let crc = ogg_crc(&p);
        p[22..26].copy_from_slice(&crc.to_le_bytes());
        p
    }

    fn head(channels: u8) -> Vec<u8> {
        let mut h = b"OpusHead".to_vec();
        h.extend_from_slice(&[1, channels]);
        h.extend_from_slice(&312u16.to_le_bytes());
        h.extend_from_slice(&48_000u32.to_le_bytes());
        h.extend_from_slice(&[0, 0, 0]);
        h
    }

    fn tags() -> Vec<u8> {
        let mut t = b"OpusTags".to_vec();
        t.extend_from_slice(&[0; 8]);
        t
    }

    /// Head, tags, then one page of `n` 3-byte packets ending at `granule`.
    fn stream(channels: u8, n: usize, granule: i64) -> Vec<u8> {
        let h = head(channels);
        let t = tags();
        let mut out = page(2, 0, 0, &[h.len() as u8], &h);
        out.extend(page(0, 0, 1, &[t.len() as u8], &t));
        out.extend(page(4, granule, 2, &vec![3; n], &vec![9; 3 * n]));
        out
    }

    #[test]
    fn parses_a_mono_stream() {
        let f = stream(1, 4, 312 + 48 * 1000);
        let o = parse_ogg_opus(&f).unwrap();
        assert_eq!(o.channels, 1);
        assert_eq!(o.pre_skip, 312);
        assert_eq!(o.packets.len(), 4);
        assert_eq!(o.duration_ms, 1000);
        assert!(matches!(o.packets[0], Cow::Borrowed(_)));
    }

    #[test]
    fn refuses_not_ogg() {
        assert_eq!(
            parse_ogg_opus(b"RIFFwhatever, not ogg at all...."),
            Err(SoundFileError::NotOgg)
        );
        assert_eq!(parse_ogg_opus(b""), Err(SoundFileError::Truncated));
    }

    #[test]
    fn refuses_bad_crc() {
        let mut f = stream(1, 4, 312 + 48_000);
        let last = f.len() - 1;
        f[last] ^= 0xff;
        assert_eq!(parse_ogg_opus(&f), Err(SoundFileError::BadChecksum));
    }

    #[test]
    fn refuses_missing_opus_head() {
        let mut h = head(1);
        h[..8].copy_from_slice(b"NotOpus!");
        let t = tags();
        let mut f = page(2, 0, 0, &[h.len() as u8], &h);
        f.extend(page(0, 0, 1, &[t.len() as u8], &t));
        f.extend(page(4, 48_000, 2, &[3], &[9; 3]));
        assert_eq!(parse_ogg_opus(&f), Err(SoundFileError::NotOpus));
    }

    #[test]
    fn refuses_too_many_channels() {
        let f = stream(6, 4, 312 + 48_000);
        assert_eq!(parse_ogg_opus(&f), Err(SoundFileError::Channels));
    }

    #[test]
    fn refuses_too_long() {
        // 6.3 s of audio.
        let f = stream(1, 4, 312 + 48 * 6300);
        assert_eq!(parse_ogg_opus(&f), Err(SoundFileError::TooLong));
        // Exactly the limit is fine.
        let f = stream(1, 4, 312 + 48 * 6200);
        assert_eq!(parse_ogg_opus(&f).unwrap().duration_ms, 6200);
        // A hostile granule must not overflow.
        let f = stream(1, 4, i64::MAX);
        assert_eq!(parse_ogg_opus(&f), Err(SoundFileError::TooLong));
    }

    #[test]
    fn refuses_empty_audio() {
        // Granule equal to pre-skip: nothing audible.
        assert_eq!(
            parse_ogg_opus(&stream(1, 4, 312)),
            Err(SoundFileError::Empty)
        );
        // No audio packets at all.
        let h = head(1);
        let t = tags();
        let mut f = page(2, 0, 0, &[h.len() as u8], &h);
        f.extend(page(4, 0, 1, &[t.len() as u8], &t));
        assert_eq!(parse_ogg_opus(&f), Err(SoundFileError::Empty));
    }

    #[test]
    fn a_hostile_negative_granule_does_not_overflow() {
        // i64::MIN minus the pre-skip would overflow (a panic in a debug build).
        assert_eq!(
            parse_ogg_opus(&stream(1, 4, i64::MIN)),
            Err(SoundFileError::Empty)
        );
    }

    #[test]
    fn refuses_far_more_packets_than_its_length_holds() {
        // Every Opus packet is at least 2.5 ms, so 100 ms holds at most 40 (plus slack of 64).
        let ok = stream(1, 104, 312 + 48 * 100);
        assert_eq!(parse_ogg_opus(&ok).unwrap().packets.len(), 104);
        let hostile = stream(1, 105, 312 + 48 * 100);
        assert_eq!(
            parse_ogg_opus(&hostile).unwrap_err().to_string(),
            "that sound file is damaged"
        );
    }

    #[test]
    fn refuses_oversize_and_truncated() {
        assert_eq!(
            parse_ogg_opus(&vec![0; MAX_BYTES + 1]),
            Err(SoundFileError::TooLarge)
        );
        let f = stream(1, 4, 312 + 48_000);
        assert_eq!(
            parse_ogg_opus(&f[..f.len() - 3]),
            Err(SoundFileError::Truncated)
        );
    }

    #[test]
    fn joins_a_packet_split_across_pages() {
        let h = head(1);
        let t = tags();
        let mut f = page(2, 0, 0, &[h.len() as u8], &h);
        f.extend(page(0, 0, 1, &[t.len() as u8], &t));
        // A 300-byte packet: 255 + 45. The first segment ends its page (granule -1: no packet ends).
        let big: Vec<u8> = (0..300u32).map(|i| i as u8).collect();
        f.extend(page(0, -1, 2, &[255], &big[..255]));
        let tail = [&big[255..], &[9, 9, 9][..]].concat();
        f.extend(page(1, 312 + 48_000, 3, &[45, 3], &tail));
        let o = parse_ogg_opus(&f).unwrap();
        assert_eq!(o.packets.len(), 2);
        assert_eq!(&*o.packets[0], &big[..]);
        assert!(matches!(o.packets[0], Cow::Owned(_)));
        assert_eq!(o.duration_ms, 1000);
    }

    #[test]
    fn names_are_trimmed_and_checked() {
        assert_eq!(clean_name("  airhorn ").as_deref(), Some("airhorn"));
        assert_eq!(clean_name(""), None);
        assert_eq!(clean_name("   "), None);
        assert_eq!(clean_name(&"a".repeat(33)), None);
        assert_eq!(clean_name(&"a".repeat(32)).map(|s| s.len()), Some(32));
        assert_eq!(clean_name("a\u{7}"), None);
    }

    #[test]
    fn emoji_is_checked() {
        assert!(valid_emoji("📯"));
        assert!(!valid_emoji("ab"));
        assert!(!valid_emoji("1"));
        assert!(!valid_emoji(" "));
        assert!(!valid_emoji(""));
        assert!(!valid_emoji("📯 📯"));
        assert!(!valid_emoji(&"📯".repeat(17)));
        assert!(valid_emoji(&"📯".repeat(16)));
    }
}
