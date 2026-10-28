//! Guessing the game a clip is of, from where it was saved.

/// Folder names that say where a clip was saved, not what it is of.
const GENERIC: [&str; 12] = [
    "desktop",
    "videos",
    "downloads",
    "documents",
    "pictures",
    "captures",
    "clips",
    "replays",
    "recordings",
    "nvidia",
    "shadowplay",
    "temp",
];

/// The longest game name the server takes.
const MAX_NAME: usize = 40;

fn usable(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.chars().count() <= MAX_NAME).then(|| name.to_owned())
}

/// `<game> YYYY.MM.DD - HH.MM.SS.cc[.DVR]` (NVIDIA's naming): the game.
fn nvidia_game(stem: &str) -> Option<String> {
    let stem = match stem.len().checked_sub(4).and_then(|i| stem.get(i..)) {
        Some(tail) if tail.eq_ignore_ascii_case(".DVR") => &stem[..stem.len() - 4],
        _ => stem,
    };
    // " 2026.05.20 - 12.26.53.03": 25 bytes; `#` is a digit
    const SHAPE: &[u8; 25] = b" ####.##.## - ##.##.##.##";
    let cut = stem.len().checked_sub(SHAPE.len())?;
    let (name, date) = (stem.get(..cut)?, stem.get(cut..)?.as_bytes());
    let fits = date.iter().zip(SHAPE).all(|(b, s)| match s {
        b'#' => b.is_ascii_digit(),
        s => b == s,
    });
    if fits { usable(name) } else { None }
}

/// The game a clip is probably of: its folder (unless generic or a drive root), then the Title tag
/// (unless it is just the file stem), then an NVIDIA-shaped file name. `None` when unsure.
pub fn guess(path: &str, title: Option<&str>) -> Option<String> {
    let mut parts: Vec<&str> = path.split(['\\', '/']).collect();
    let file = parts.pop().unwrap_or("");
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let folder = parts
        .pop()
        .filter(|f| !f.is_empty() && !f.ends_with(':'))
        .filter(|f| !GENERIC.contains(&f.to_lowercase().as_str()) && *f != "tmp");
    folder
        .and_then(usable)
        .or_else(|| title.map(str::trim).filter(|t| *t != stem).and_then(usable))
        .or_else(|| nvidia_game(stem))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NVIDIA_FILE: &str = "RUST 2026.05.20 - 12.26.53.03.DVR.mp4";

    #[test]
    fn folder_names_the_game() {
        assert_eq!(
            guess(
                r"C:\Users\x\Videos\Overwatch 2\Overwatch 2 2026.05.20 - 12.26.53.03.DVR.mp4",
                None
            )
            .as_deref(),
            Some("Overwatch 2")
        );
        assert_eq!(
            guess("/home/x/Videos/Valorant/a.mp4", None).as_deref(),
            Some("Valorant")
        );
    }

    #[test]
    fn nvidia_file_name_names_the_game_when_nothing_better() {
        let p = format!(r"C:\Users\x\Desktop\{NVIDIA_FILE}");
        assert_eq!(guess(&p, None).as_deref(), Some("RUST"));
        // the Title tag comes before the file name
        assert_eq!(guess(&p, Some("Rust")).as_deref(), Some("Rust"));
        // forward slashes work too
        assert_eq!(
            guess(&format!("C:/Users/x/Desktop/{NVIDIA_FILE}"), None).as_deref(),
            Some("RUST")
        );
    }

    #[test]
    fn generic_folders_and_drive_roots_give_nothing() {
        assert_eq!(guess(r"C:\Users\x\Desktop\funny.mp4", None), None);
        assert_eq!(guess(r"C:\Users\x\Videos\clip.mp4", None), None);
        assert_eq!(guess(r"D:\clip.mp4", None), None);
        assert_eq!(guess("/clip.mp4", None), None);
        assert_eq!(guess(r"C:\Users\x\DOWNLOADS\clip.mp4", None), None);
        assert_eq!(
            guess(r"C:\Users\x\Videos\NVIDIA\Captures\a.mp4", None),
            None
        );
    }

    #[test]
    fn a_title_equal_to_the_file_stem_is_not_a_game() {
        assert_eq!(guess(r"C:\Users\x\Desktop\funny.mp4", Some("funny")), None);
        assert_eq!(
            guess(r"C:\Users\x\Desktop\funny.mp4", Some("  Apex Legends ")).as_deref(),
            Some("Apex Legends")
        );
        assert_eq!(guess(r"C:\Users\x\Desktop\funny.mp4", Some("  ")), None);
    }

    #[test]
    fn only_nvidia_shaped_file_names_are_guessed() {
        assert_eq!(guess(r"C:\Users\x\Desktop\VID 20260101.mp4", None), None);
        assert_eq!(
            guess(
                r"C:\Users\x\Desktop\Apex 2026.05.20 - 12.26.53.03.mp4",
                None
            )
            .as_deref(),
            Some("Apex")
        );
        assert_eq!(
            guess(r"C:\Users\x\Desktop\ 2026.05.20 - 12.26.53.03.mp4", None),
            None
        );
    }

    #[test]
    fn names_over_40_characters_are_skipped() {
        let long = "x".repeat(41);
        assert_eq!(guess(&format!("/v/{long}/a.mp4"), None), None);
        assert_eq!(
            guess(&format!("/v/{long}/a.mp4"), Some("Fallback")).as_deref(),
            Some("Fallback")
        );
    }
}
