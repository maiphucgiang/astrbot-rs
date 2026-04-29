use crate::errors::{AstrBotError, Result};
use std::path::Path;

/// Ensure a directory exists
pub async fn ensure_dir<P: AsRef<Path>>(path: P) -> Result<()> {
    let path = path.as_ref();
    if !path.exists() {
        tokio::fs::create_dir_all(path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("failed to create directory: {}", e)))?;
    }
    Ok(())
}

/// Read a file to string
pub async fn read_string<P: AsRef<Path>>(path: P) -> Result<String> {
    tokio::fs::read_to_string(path.as_ref())
        .await
        .map_err(|e| AstrBotError::Internal(format!("failed to read file: {}", e)))
}

/// Write a string to file
pub async fn write_string<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
    tokio::fs::write(path.as_ref(), content)
        .await
        .map_err(|e| AstrBotError::Internal(format!("failed to write file: {}", e)))
}
