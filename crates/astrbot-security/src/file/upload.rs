use anyhow::{bail, Result};
use std::path::Path;

static ALLOWED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "mp4", "mp3", "pdf", "txt", "json",
];
const MAX_FILE_SIZE: usize = 10 * 1024 * 1024; // 10MB

pub struct SafeFileStorage {
    pub base_dir: std::path::PathBuf,
}

impl SafeFileStorage {
    pub fn save(&self, original_name: &str, bytes: &[u8]) -> Result<String> {
        if bytes.len() > MAX_FILE_SIZE {
            bail!(
                "File too large: {} bytes (max {})",
                bytes.len(),
                MAX_FILE_SIZE
            );
        }

        let ext = Path::new(original_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_lowercase();

        if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
            bail!("Forbidden file extension: {}", ext);
        }

        // Magic number check
        if let Some(kind) = infer::get(bytes) {
            let real_ext = kind.extension();
            if real_ext != ext && !is_extension_alias(&ext, real_ext) {
                bail!(
                    "File type mismatch: claimed .{} but detected .{}",
                    ext,
                    real_ext
                );
            }
        }

        let file_id = uuid::Uuid::new_v4().to_string();
        let stored_name = format!("{}.{}", file_id, ext);
        let subdir = &file_id[..2];
        let dir = self.base_dir.join(subdir);
        std::fs::create_dir_all(&dir)?;

        let final_path = dir.join(&stored_name);
        std::fs::write(&final_path, bytes)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(file_id)
    }

    /// Load by file_id (must be resolved through database mapping — path never exposed to user)
    pub fn load(&self, file_id: &str, resolved_path: &std::path::Path) -> Result<Vec<u8>> {
        let canonical = std::fs::canonicalize(resolved_path)?;
        let base_canonical = std::fs::canonicalize(&self.base_dir)?;

        if !canonical.starts_with(&base_canonical) {
            bail!("Path traversal attempt detected: {}", file_id);
        }

        Ok(std::fs::read(canonical)?)
    }
}

fn is_extension_alias(claimed: &str, detected: &str) -> bool {
    matches!(
        (claimed, detected),
        ("jpg", "jpeg") | ("jpeg", "jpg") | ("txt", "text") | ("mp4", "m4v")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_txt() {
        let dir = std::env::temp_dir().join("astrbot_test_files");
        let _ = std::fs::remove_dir_all(&dir);
        let storage = SafeFileStorage {
            base_dir: dir.clone(),
        };
        let id = storage.save("test.txt", b"hello world").unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn test_forbidden_extension() {
        let dir = std::env::temp_dir().join("astrbot_test_files");
        let storage = SafeFileStorage { base_dir: dir };
        let result = storage.save("test.exe", b"malicious");
        assert!(result.is_err());
    }
}
