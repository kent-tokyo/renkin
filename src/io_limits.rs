//! Bounded readers for attacker-controlled local text files.

use std::io::Read;

use anyhow::{Context, Result, bail};

/// Maximum size accepted for a caller-supplied text file.
pub const MAX_TEXT_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Read UTF-8 text from an arbitrary reader with the same hard byte cap used
/// for caller-supplied files. This is also used for stdin, where metadata
/// cannot provide an upfront size check.
pub fn read_bounded_reader(reader: impl Read, label: &str) -> Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_TEXT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {label}"))?;
    if bytes.len() as u64 > MAX_TEXT_FILE_BYTES {
        bail!(
            "resource_exhausted: {label} exceeds {} bytes",
            MAX_TEXT_FILE_BYTES
        );
    }
    String::from_utf8(bytes).with_context(|| format!("{label} is not valid UTF-8"))
}

/// Open and read one regular, non-symlink UTF-8 text file with a hard byte cap.
/// The cap is enforced on the open file descriptor as well as its initial
/// metadata, so a file replacement or growth cannot turn the reader into an
/// unbounded allocation.
pub fn read_bounded_text_file(path: &str, label: &str) -> Result<String> {
    let link_metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {label} {path}"))?;
    if link_metadata.file_type().is_symlink() {
        bail!("{label} {path:?} must not be a symlink");
    }
    let file =
        std::fs::File::open(path).with_context(|| format!("failed to read {label} {path}"))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect {label} {path}"))?;
    if !metadata.is_file() {
        bail!("{label} {path:?} is not a regular file");
    }
    if metadata.len() > MAX_TEXT_FILE_BYTES {
        bail!(
            "resource_exhausted: {label} exceeds {} bytes",
            MAX_TEXT_FILE_BYTES
        );
    }
    read_bounded_reader(file, &format!("{label} {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::Cursor;

    #[test]
    fn rejects_oversized_file_before_allocating_contents() {
        let path = std::env::temp_dir().join(format!(
            "renkin-bounded-reader-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .expect("create sparse test file");
        file.set_len(MAX_TEXT_FILE_BYTES + 1)
            .expect("grow sparse test file");
        let result = read_bounded_text_file(path.to_str().unwrap(), "test file");
        let _ = std::fs::remove_file(&path);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("resource_exhausted")
        );
    }

    #[test]
    fn bounds_reader_without_file_metadata() {
        let input = vec![b'x'; (MAX_TEXT_FILE_BYTES + 1) as usize];
        let result = read_bounded_reader(Cursor::new(input), "stdin input");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("resource_exhausted")
        );
    }
}
