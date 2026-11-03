//! Uploaded document storage.
//!
//! v1 implements the `local` provider: files live under `STORAGE_PATH` in a
//! per-tender directory with a random prefix. Keys are always validated
//! (no `..`, no absolute paths) before touching the filesystem.
//!
//! `STORAGE_PROVIDER=s3` is intentionally **not** implemented in v1 — the
//! service refuses to start that way instead of pretending to store files.

use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use tender_common::config::{Config, ConfigError};
use tender_common::error::{AppError, AppResult};

/// Accepted upload types, keyed by lowercase extension.
const ALLOWED_EXTENSIONS: [&str; 6] = ["pdf", "docx", "xlsx", "txt", "rtf", "odt"];

pub struct StoredFile {
    pub key: String,
    pub size: u64,
    pub sha256: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Validate size, extension and the magic bytes we can check cheaply.
pub fn validate_upload(max_file_size: u64, filename: &str, bytes: &[u8]) -> AppResult<()> {
    if bytes.is_empty() {
        return Err(AppError::Validation("uploaded file is empty".into()));
    }
    if bytes.len() as u64 > max_file_size {
        return Err(AppError::Validation(format!(
            "file exceeds the maximum upload size of {max_file_size} bytes"
        )));
    }
    let extension = filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
        return Err(AppError::Validation(format!(
            "unsupported file type '.{extension}'; allowed: {}",
            ALLOWED_EXTENSIONS.join(", ")
        )));
    }
    let magic_ok = match extension.as_str() {
        "pdf" => bytes.starts_with(b"%PDF"),
        // docx/xlsx/odt are ZIP containers.
        "docx" | "xlsx" | "odt" => bytes.starts_with(b"PK\x03\x04"),
        _ => true,
    };
    if !magic_ok {
        return Err(AppError::Validation(format!(
            "file content does not match the '.{extension}' extension"
        )));
    }
    Ok(())
}

/// Keep only a safe suffix of the original name for the stored key.
fn sanitize_stem(filename: &str) -> String {
    let stem = filename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(filename);
    let cleaned: String = stem
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();
    trimmed.chars().take(40).collect()
}

fn extension(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_else(|| "bin".to_string())
}

pub async fn store_upload(
    cfg: &Config,
    tender_id: Uuid,
    filename: &str,
    bytes: &[u8],
) -> AppResult<StoredFile> {
    validate_upload(cfg.max_file_size, filename, bytes)?;
    match cfg.storage_provider.as_str() {
        "local" => {
            let directory = PathBuf::from(&cfg.storage_path).join(tender_id.to_string());
            tokio::fs::create_dir_all(&directory).await?;
            let stem = sanitize_stem(filename);
            let key = format!(
                "{tender_id}/{}-{}.{}",
                Uuid::new_v4().simple(),
                if stem.is_empty() { "upload" } else { &stem },
                extension(filename)
            );
            let path = PathBuf::from(&cfg.storage_path).join(&key);
            tokio::fs::write(&path, bytes).await?;
            Ok(StoredFile {
                key,
                size: bytes.len() as u64,
                sha256: sha256_hex(bytes),
            })
        }
        other => Err(AppError::Config(ConfigError(format!(
            "STORAGE_PROVIDER='{other}' is not implemented in v1; use STORAGE_PROVIDER=local"
        )))),
    }
}

/// Resolve a storage key to an on-disk path, rejecting traversal.
pub fn resolve_path(base: &Path, key: &str) -> AppResult<PathBuf> {
    let relative = Path::new(key);
    for component in relative.components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err(AppError::Validation(format!("invalid storage key '{key}'"))),
        }
    }
    Ok(base.join(relative))
}

pub async fn read_file(cfg: &Config, key: &str) -> AppResult<Vec<u8>> {
    let base = PathBuf::from(&cfg.storage_path);
    let path = resolve_path(&base, key)?;
    tokio::fs::read(&path)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound(format!("file '{key}'")),
            _ => AppError::Io(err),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_SIZE: u64 = 1024;

    #[test]
    fn rejects_empty_and_oversized_uploads() {
        assert!(validate_upload(MAX_SIZE, "spec.pdf", b"").is_err());
        let big = vec![0u8; 2048];
        assert!(validate_upload(MAX_SIZE, "spec.pdf", &big).is_err());
    }

    #[test]
    fn rejects_unknown_extensions() {
        assert!(validate_upload(MAX_SIZE, "run.exe", b"MZ").is_err());
        assert!(validate_upload(MAX_SIZE, "page.html", b"<html>").is_err());
    }

    #[test]
    fn validates_magic_bytes() {
        assert!(validate_upload(MAX_SIZE, "spec.pdf", b"%PDF-1.7 body").is_ok());
        assert!(validate_upload(MAX_SIZE, "spec.pdf", b"not a pdf").is_err());
        assert!(validate_upload(MAX_SIZE, "spec.docx", b"PK\x03\x04body").is_ok());
        assert!(validate_upload(MAX_SIZE, "spec.xlsx", b"%PDF").is_err());
    }

    #[test]
    fn storage_keys_reject_traversal() {
        let base = Path::new("/tmp/ts-storage");
        assert!(resolve_path(base, "../etc/passwd").is_err());
        assert!(resolve_path(base, "/etc/passwd").is_err());
        assert!(resolve_path(base, "tender/file.pdf").is_ok());
    }
}
