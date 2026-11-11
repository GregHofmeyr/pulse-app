//! Files queued in the composer (#57). A staged file is ready to upload: an image already
//! converted to WebP (in the staging folder), anything else the user's own file by path. The UI
//! holds only each one's id and what its chip shows.

pub mod commands;
pub mod prepare;
pub mod send;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use pulse_protocol::rest::AttachmentKind;
use serde::Serialize;

/// Why a file can't be queued, in the words the composer shows.
pub const FOLDER: &str = "Folders can't be sent as they are. Zip the folder first.";
pub const UNREADABLE: &str = "That file couldn't be read.";
pub const EMPTY: &str = "That file is empty.";

/// A queued file, as the UI sees it (mirrored by hand as `StagedFile` in lib/tauri.ts).
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Staged {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub content_type: String,
    pub kind: AttachmentKind,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// `data:image/png;base64,…`, a small thumbnail of an image.
    pub preview: Option<String>,
    /// What gets uploaded.
    #[serde(skip)]
    pub path: PathBuf,
    /// The file is ours (in the staging folder) and is deleted with the entry.
    #[serde(skip)]
    pub owned: bool,
}

/// One result per file asked for: staged, or why not.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum StageResult {
    Staged { file: Staged },
    Refused { name: String, error: String },
}

/// The staged files and the folder converted images are written to.
pub struct Stager {
    dir: PathBuf,
    files: Mutex<HashMap<String, Staged>>,
}

fn new_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into())
}

impl Stager {
    /// Staging in `dir`, emptied now: anything there is from an earlier run.
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::remove_dir_all(&dir);
        Self {
            dir,
            files: Mutex::new(HashMap::new()),
        }
    }

    fn keep(&self, s: Staged) -> Staged {
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(s.id.clone(), s.clone());
        s
    }

    /// An image converted to WebP, written to the staging folder.
    fn stage_image(&self, name: &str, out: prepare::WebpOut) -> Result<Staged, String> {
        let id = new_id();
        std::fs::create_dir_all(&self.dir).map_err(|_| UNREADABLE.to_string())?;
        let path = self.dir.join(&id);
        std::fs::write(&path, &out.bytes).map_err(|_| UNREADABLE.to_string())?;
        Ok(self.keep(Staged {
            id,
            name: prepare::webp_name(name),
            size: out.bytes.len() as u64,
            content_type: "image/webp".into(),
            kind: AttachmentKind::Image,
            width: Some(out.width),
            height: Some(out.height),
            preview: Some(format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&out.preview_png)
            )),
            path,
            owned: true,
        }))
    }

    fn plain(&self, name: String, size: u64, path: PathBuf, owned: bool) -> Staged {
        let (content_type, kind) = prepare::content_type_for(&name);
        self.keep(Staged {
            id: new_id(),
            name,
            size,
            content_type: content_type.into(),
            kind,
            width: None,
            height: None,
            preview: None,
            path,
            owned,
        })
    }

    /// Queue a file the user picked or dropped. Blocking (an image is converted): call from
    /// `spawn_blocking`.
    pub fn stage_path(&self, path: &Path) -> Result<Staged, String> {
        let meta = std::fs::metadata(path).map_err(|_| UNREADABLE.to_string())?;
        if meta.is_dir() {
            return Err(FOLDER.into());
        }
        if meta.len() == 0 {
            return Err(EMPTY.into());
        }
        let name = display_name(path);
        // Images are read whole to convert; a huge "image" goes as a file without being read.
        if prepare::is_image_name(&name) && meta.len() <= IMAGE_READ_MAX {
            let bytes = std::fs::read(path).map_err(|_| UNREADABLE.to_string())?;
            if let Ok(out) = prepare::to_webp(&bytes) {
                return self.stage_image(&name, out);
            }
        }
        Ok(self.plain(name, meta.len(), path.to_path_buf(), false))
    }

    /// Queue pasted bytes (a screenshot from the clipboard), written to the staging folder unless
    /// they convert.
    pub fn stage_bytes(&self, name: &str, bytes: &[u8]) -> Result<Staged, String> {
        if bytes.is_empty() {
            return Err(EMPTY.into());
        }
        let name = match Path::new(name).file_name() {
            Some(n) if !name.trim().is_empty() => n.to_string_lossy().into_owned(),
            _ => "pasted".into(),
        };
        if let Ok(out) = prepare::to_webp(bytes) {
            return self.stage_image(&name, out);
        }
        std::fs::create_dir_all(&self.dir).map_err(|_| UNREADABLE.to_string())?;
        let path = self.dir.join(new_id());
        std::fs::write(&path, bytes).map_err(|_| UNREADABLE.to_string())?;
        Ok(self.plain(name, bytes.len() as u64, path, true))
    }

    pub fn get(&self, id: &str) -> Option<Staged> {
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }

    /// Forget a staged file, deleting it if it is ours.
    pub fn remove(&self, id: &str) {
        let gone = self
            .files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
        if let Some(s) = gone
            && s.owned
        {
            let _ = std::fs::remove_file(&s.path);
        }
    }
}

/// The biggest file read whole to try converting it as an image.
const IMAGE_READ_MAX: u64 = 200 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([1, 2, 3, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let st = Stager::new(dir.path().join("staging"));
        assert_eq!(st.stage_path(dir.path()).unwrap_err(), FOLDER);
    }

    #[test]
    fn an_image_is_converted_and_a_pdf_staged_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let st = Stager::new(dir.path().join("staging"));
        let shot = dir.path().join("Shot.png");
        std::fs::write(&shot, png(20, 10)).unwrap();
        let img = st.stage_path(&shot).unwrap();
        assert_eq!(img.name, "Shot.webp");
        assert_eq!(img.content_type, "image/webp");
        assert_eq!((img.width, img.height), (Some(20), Some(10)));
        assert!(img.owned && img.path.starts_with(dir.path().join("staging")));
        assert!(
            img.preview
                .as_deref()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(std::fs::metadata(&img.path).unwrap().len(), img.size);

        let pdf = dir.path().join("report.pdf");
        std::fs::write(&pdf, b"%PDF-1.4 hello").unwrap();
        let file = st.stage_path(&pdf).unwrap();
        assert_eq!((file.name.as_str(), file.size), ("report.pdf", 14));
        assert_eq!(file.kind, AttachmentKind::File);
        assert_eq!(file.path, pdf);
        assert!(!file.owned);

        // removing deletes only our own copy
        st.remove(&img.id);
        st.remove(&file.id);
        assert!(!img.path.exists());
        assert!(pdf.exists());
        assert!(st.get(&img.id).is_none());
    }

    #[test]
    fn a_broken_png_goes_as_a_file_and_empty_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let st = Stager::new(dir.path().join("staging"));
        let bad = dir.path().join("broken.png");
        std::fs::write(&bad, b"not really a png").unwrap();
        let s = st.stage_path(&bad).unwrap();
        assert_eq!(
            (s.name.as_str(), s.kind),
            ("broken.png", AttachmentKind::File)
        );
        assert_eq!(s.content_type, "application/octet-stream");
        let empty = dir.path().join("empty.txt");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(st.stage_path(&empty).unwrap_err(), EMPTY);
    }

    #[test]
    fn pasted_bytes_are_staged() {
        let dir = tempfile::tempdir().unwrap();
        let st = Stager::new(dir.path().join("staging"));
        let s = st.stage_bytes("image.png", &png(8, 8)).unwrap();
        assert_eq!(s.name, "image.webp");
        assert!(s.path.exists());
        let t = st.stage_bytes("../notes.txt", b"hi").unwrap();
        assert_eq!((t.name.as_str(), t.size), ("notes.txt", 2));
        assert!(t.owned && t.path.exists());
        assert_eq!(st.stage_bytes("x", b"").unwrap_err(), EMPTY);
    }

    #[test]
    fn a_new_stager_clears_an_earlier_runs_files() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("old"), b"x").unwrap();
        let _st = Stager::new(staging.clone());
        assert!(!staging.join("old").exists());
    }
}
