//! "Zip and send" (#57): a dropped folder can't go as it is, so it is zipped here first.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

/// The biggest zip that can be sent (a temporary send's limit).
pub const MAX_ZIP_BYTES: u64 = pulse_protocol::rest::attachment_limits::TEMP_MAX_BYTES;
pub const TOO_BIG: &str = "That folder is over 2 GB zipped. Send fewer files.";
pub const EMPTY_FOLDER: &str = "That folder is empty.";

/// The zip file being written, counting its bytes so a folder too big to send stops as soon as
/// the zip passes the limit, even in the middle of one huge file (never filling the disk).
struct Counted {
    file: std::fs::File,
    /// Where the next write lands (the zip writer seeks back to fill in headers).
    pos: u64,
    limit: u64,
}

impl Write for Counted {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.pos + buf.len() as u64 > self.limit {
            return Err(std::io::Error::other(TOO_BIG));
        }
        let n = self.file.write(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl std::io::Seek for Counted {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.pos = self.file.seek(pos)?;
        Ok(self.pos)
    }
}

/// Zip `folder` into `dest`, its own name as the zip's top folder (as file managers do), with its
/// files and folders inside by their paths (symbolic links and special files skipped). Fails,
/// removing `dest`, once the zip would pass [`MAX_ZIP_BYTES`].
pub fn zip_folder(folder: &Path, dest: &Path) -> Result<u64, String> {
    zip_folder_within(folder, dest, MAX_ZIP_BYTES)
}

/// Whether an error came from [`Counted`] refusing to grow past the limit.
fn past_limit(e: &dyn std::fmt::Display) -> String {
    if e.to_string().contains(TOO_BIG) {
        TOO_BIG.to_string()
    } else {
        super::UNREADABLE.to_string()
    }
}

fn zip_folder_within(folder: &Path, dest: &Path, limit: u64) -> Result<u64, String> {
    let unreadable = |_| super::UNREADABLE.to_string();
    let file = std::fs::File::create(dest).map_err(unreadable)?;
    let root = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "folder".into());
    let result = (|| {
        let mut zip = zip::ZipWriter::new(Counted {
            file,
            pos: 0,
            limit,
        });
        // zip64 only where a file could need it (some older tools dislike it everywhere)
        let options = |big: bool| {
            SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .large_file(big)
        };
        zip.add_directory(format!("{root}/"), options(false))
            .map_err(|e| past_limit(&e))?;
        let mut files = 0usize;
        let mut stack: Vec<PathBuf> = vec![folder.to_path_buf()];
        let mut buf = vec![0u8; 1024 * 1024];
        while let Some(dir) = stack.pop() {
            // An entry that can't be read fails the zip: never a zip quietly missing files.
            let mut entries: Vec<_> = std::fs::read_dir(&dir)
                .map_err(unreadable)?
                .collect::<Result<_, _>>()
                .map_err(unreadable)?;
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let path = e.path();
                // symlink_metadata: a link isn't followed (it could point anywhere, or loop)
                let meta = std::fs::symlink_metadata(&path).map_err(unreadable)?;
                let rel = path
                    .strip_prefix(folder)
                    .map_err(|_| super::UNREADABLE.to_string())?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let name = format!("{root}/{rel}");
                if meta.is_dir() {
                    zip.add_directory(format!("{name}/"), options(false))
                        .map_err(|e| past_limit(&e))?;
                    stack.push(path);
                } else if meta.is_file() {
                    zip.start_file(name, options(meta.len() >= u32::MAX as u64))
                        .map_err(|e| past_limit(&e))?;
                    let mut f = std::fs::File::open(&path).map_err(unreadable)?;
                    loop {
                        let n = f.read(&mut buf).map_err(unreadable)?;
                        if n == 0 {
                            break;
                        }
                        zip.write_all(&buf[..n]).map_err(|e| past_limit(&e))?;
                    }
                    files += 1;
                }
            }
        }
        if files == 0 {
            return Err(EMPTY_FOLDER.to_string());
        }
        let out = zip.finish().map_err(|e| past_limit(&e))?;
        out.file.metadata().map(|m| m.len()).map_err(unreadable)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zips_a_folder_with_nested_files_under_its_own_name() {
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
        assert_eq!(
            names,
            [
                "world/",
                "world/level.dat",
                "world/region/",
                "world/region/r.0.0.mca"
            ]
        );
        let mut body = Vec::new();
        z.by_name("world/region/r.0.0.mca")
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

    #[test]
    fn a_zip_past_the_limit_stops_in_the_middle_of_a_file_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big");
        std::fs::create_dir_all(&big).unwrap();
        // noisy bytes barely compress: 3 MB of them pass a 1 MB limit part-way through the file
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let noise: Vec<u8> = (0..3_000_000)
            .map(|_| {
                // xorshift64: no pattern deflate can use
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        std::fs::write(big.join("vm.img"), noise).unwrap();
        let dest = dir.path().join("big.zip");
        assert_eq!(
            zip_folder_within(&big, &dest, 1_000_000).unwrap_err(),
            TOO_BIG
        );
        assert!(!dest.exists());
    }
}
