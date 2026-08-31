//! Bounded readers for attacker-controlled local text files.

use std::io::{BufRead, Read};
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Maximum size accepted for a caller-supplied text file.
pub const MAX_TEXT_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_TEXT_LINE_BYTES: usize = 64 * 1024;

/// Read one UTF-8 line without allocating beyond the shared line cap. An
/// oversized line is consumed through its newline before returning an error,
/// so a streaming caller can safely continue with the following record.
pub fn read_bounded_line(reader: &mut impl BufRead, label: &str) -> Result<Option<String>> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let buffer = reader
            .fill_buf()
            .with_context(|| format!("failed to read {label}"))?;
        if buffer.is_empty() {
            if bytes.is_empty() && !oversized {
                return Ok(None);
            }
            break;
        }
        let newline = buffer.iter().position(|&byte| byte == b'\n');
        let consumed = newline.map_or(buffer.len(), |position| position + 1);
        if !oversized {
            let remaining = MAX_TEXT_LINE_BYTES.saturating_sub(bytes.len());
            let copy_len = consumed.min(remaining);
            bytes.extend_from_slice(&buffer[..copy_len]);
            if copy_len < consumed {
                oversized = true;
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    if oversized {
        bail!(
            "resource_exhausted: {label} line exceeds {} bytes",
            MAX_TEXT_LINE_BYTES
        );
    }
    String::from_utf8(bytes)
        .with_context(|| format!("{label} line is not valid UTF-8"))
        .map(Some)
}

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
    read_bounded_text_path(path, label)
}

/// Path-generic variant used by APIs that accept Path values directly.
pub fn read_bounded_text_path(path: impl AsRef<Path>, label: &str) -> Result<String> {
    let path_display = path.as_ref().display().to_string();
    let bytes = read_bounded_bytes_path(path, label)?;
    String::from_utf8(bytes).with_context(|| format!("{label} {path_display} is not valid UTF-8"))
}

/// Open and read one regular, non-symlink file with a hard byte cap.
pub fn read_bounded_bytes_file(path: &str, label: &str) -> Result<Vec<u8>> {
    read_bounded_bytes_path(path, label)
}

/// Path-generic byte reader for binary artifacts and provenance hashing.
pub fn read_bounded_bytes_path(path: impl AsRef<Path>, label: &str) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let link_metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {label} {}", path.display()))?;
    if link_metadata.file_type().is_symlink() {
        bail!("{label} {path:?} must not be a symlink");
    }
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to read {label} {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect {label} {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{label} {path:?} is not a regular file");
    }
    if metadata.len() > MAX_TEXT_FILE_BYTES {
        bail!(
            "resource_exhausted: {label} exceeds {} bytes",
            MAX_TEXT_FILE_BYTES
        );
    }
    let mut bytes = Vec::new();
    file.take(MAX_TEXT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {label} {}", path.display()))?;
    if bytes.len() as u64 > MAX_TEXT_FILE_BYTES {
        bail!(
            "resource_exhausted: {label} exceeds {} bytes",
            MAX_TEXT_FILE_BYTES
        );
    }
    Ok(bytes)
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

    #[test]
    fn bounded_line_rejects_large_line_and_preserves_following_line() {
        let input = format!("{}\nCCO\n", "x".repeat(MAX_TEXT_LINE_BYTES + 1));
        let mut reader = std::io::Cursor::new(input.into_bytes());
        let error = read_bounded_line(&mut reader, "stdin").unwrap_err();
        assert!(error.to_string().contains("line exceeds"));
        assert_eq!(
            read_bounded_line(&mut reader, "stdin").unwrap(),
            Some("CCO\n".into())
        );
    }
}
