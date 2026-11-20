//! What every file Pulse saves from the network gets: a name with no bidirectional controls (which can make
//! `invoice<U+202E>fdp.exe` read as `invoiceexe.pdf` in a Save dialog or a file manager), and on Windows the
//! Mark-of-the-Web (a `Zone.Identifier` stream naming the Internet zone), so SmartScreen and Office's Protected View
//! treat it as downloaded, as they would a browser's download.

use std::path::Path;

/// The bidirectional formatting characters: the marks (LRM, RLM, ALM), the embeddings and overrides (LRE, RLE, PDF,
/// LRO, RLO) and the isolates (LRI, RLI, FSI, PDI). None belongs in a file name.
pub fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// The Mark-of-the-Web's contents: the Internet zone, with the app's site as where it came from (never the file's
/// own signed link).
#[cfg_attr(not(windows), allow(dead_code))]
const ZONE_IDENTIFIER: &str = "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://pulsechat.co.za/\r\n";

/// Marks a file just downloaded to `path` as from the Internet (Windows: the NTFS `Zone.Identifier` stream; elsewhere
/// nothing). Best effort: a volume without streams (FAT32, a network share) is logged and the save stands.
pub async fn mark_downloaded(path: &Path) {
    #[cfg(windows)]
    if let Err(e) = write_zone_identifier(path).await {
        tracing::warn!(error = %e, "saved file: no Mark-of-the-Web");
    }
    #[cfg(not(windows))]
    let _ = path;
}

#[cfg(windows)]
async fn write_zone_identifier(path: &Path) -> std::io::Result<()> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    tokio::fs::write(stream, ZONE_IDENTIFIER).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bidi_controls_are_named() {
        for c in [
            '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
            '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            assert!(is_bidi_control(c), "{:04X}", c as u32);
        }
        for c in ['a', '.', ' ', '\u{200B}', '\u{2029}', '\u{206A}', 'é', 'ש'] {
            assert!(!is_bidi_control(c), "{:04X}", c as u32);
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_saved_file_carries_the_mark_of_the_web() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.pdf");
        std::fs::write(&file, b"%PDF").unwrap();
        mark_downloaded(&file).await;
        let mut stream = file.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        assert_eq!(std::fs::read_to_string(stream).unwrap(), ZONE_IDENTIFIER);
        // the file itself is untouched
        assert_eq!(std::fs::read(&file).unwrap(), b"%PDF");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn elsewhere_marking_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.pdf");
        std::fs::write(&file, b"%PDF").unwrap();
        mark_downloaded(&file).await;
        assert_eq!(std::fs::read(&file).unwrap(), b"%PDF");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
