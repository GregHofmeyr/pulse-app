//! "Zip and send" (#57): a dropped folder can't go as it is, so it is zipped here first.

use std::io::Write;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

/// The biggest zip that can be sent (a temporary send's limit).
pub const MAX_ZIP_BYTES: u64 = pulse_protocol::rest::attachment_limits::TEMP_MAX_BYTES;
pub const TOO_BIG: &str = "That folder is over 2 GB zipped. Send fewer files.";
pub const EMPTY_FOLDER: &str = "That folder is empty.";

/// Zip `folder` (its files and folders inside, by their paths under it; symbolic links skipped)
/// into `dest`. Fails, removing `dest`, once the zip grows past [`MAX_ZIP_BYTES`].
pub fn zip_folder(folder: &Path, dest: &Path) -> Result<u64, String> {
    let out = std::fs::File::create(dest).map_err(|_| super::UNREADABLE.to_string())?;
    let result = (|| {
        let mut zip = zip::ZipWriter::new(out);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(true);
        let mut files = 0usize;
        let mut stack: Vec<PathBuf> = vec![folder.to_path_buf()];
        let mut buf = vec![0u8; 1024 * 1024];
        while let Some(dir) = stack.pop() {
            let mut entries: Vec<_> = std::fs::read_dir(&dir)
                .map_err(|_| super::UNREADABLE.to_string())?
                .filter_map(Result::ok)
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let path = e.path();
                // symlink_metadata: a link isn't followed (it could point anywhere, or loop)
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                let rel = path
                    .strip_prefix(folder)
                    .map_err(|_| super::UNREADABLE.to_string())?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                if meta.is_dir() {
                    zip.add_directory(format!("{rel}/"), options)
                        .map_err(|_| super::UNREADABLE.to_string())?;
                    stack.push(path);
                } else if meta.is_file() {
                    zip.start_file(rel, options)
                        .map_err(|_| super::UNREADABLE.to_string())?;
                    let mut f =
                        std::fs::File::open(&path).map_err(|_| super::UNREADABLE.to_string())?;
                    loop {
                        let n = std::io::Read::read(&mut f, &mut buf)
                            .map_err(|_| super::UNREADABLE.to_string())?;
                        if n == 0 {
                            break;
                        }
                        zip.write_all(&buf[..n])
                            .map_err(|_| super::UNREADABLE.to_string())?;
                    }
                    files += 1;
                    // Checked as it grows: a huge folder stops early instead of filling the disk.
                    if std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0) > MAX_ZIP_BYTES {
                        return Err(TOO_BIG.to_string());
                    }
                }
            }
        }
        if files == 0 {
            return Err(EMPTY_FOLDER.to_string());
        }
        zip.finish().map_err(|_| super::UNREADABLE.to_string())?;
        let size = std::fs::metadata(dest)
            .map_err(|_| super::UNREADABLE.to_string())?
            .len();
        if size > MAX_ZIP_BYTES {
            return Err(TOO_BIG.to_string());
        }
        Ok(size)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn zips_a_folder_with_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        let world = dir.path().join("world");
        std::fs::create_dir_all(world.join("region")).unwrap();
        std::fs::write(world.join("level.dat"), b"level").unwrap();
        std::fs::write(world.join("region").join("r.0.0.mca"), vec![7u8; 5000]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", world.join("link")).unwrap();
        let dest = dir.path().join("world.zip");
        let size = zip_folder(&world, &dest).unwrap();
        assert_eq!(size, std::fs::metadata(&dest).unwrap().len());
        let mut z = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
        let mut names: Vec<String> = z.file_names().map(|n| n.unwrap().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["level.dat", "region/", "region/r.0.0.mca"]);
        let mut body = Vec::new();
        z.by_name("region/r.0.0.mca")
            .unwrap()
            .read_to_end(&mut body)
            .unwrap();
        assert_eq!(body, vec![7u8; 5000]);
    }

    #[test]
    fn an_empty_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        std::fs::create_dir_all(empty.join("nothing")).unwrap();
        let dest = dir.path().join("e.zip");
        assert_eq!(zip_folder(&empty, &dest).unwrap_err(), EMPTY_FOLDER);
        assert!(!dest.exists());
    }
}
