//! Atomic write of `auto-persona.md` (memory-system-spec §4).
//!
//! "Atomic" matters because both the chat command and the Settings UI
//! read the file concurrently with the writer. Worst case without
//! atomicity is the chat seeing a half-written file; with the
//! tempfile + rename dance the reader either sees the old or the new
//! content, never a torn one.

use std::io::Write;
use std::path::Path;

use crate::error::{CorivoError, Result};

pub const AUTO_PERSONA_FILE: &str = "auto-persona.md";

pub fn atomic_write(app_data_dir: &Path, content: &str) -> Result<()> {
    let final_path = app_data_dir.join(AUTO_PERSONA_FILE);
    let tmp_path = final_path.with_extension("md.partial");
    if let Some(parent) = final_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CorivoError::Internal(format!("auto-persona.mkdir failed: {e}")))?;
    }
    {
        let mut f = std::fs::File::create(&tmp_path)
            .map_err(|e| CorivoError::Internal(format!("auto-persona.tmp create failed: {e}")))?;
        f.write_all(content.as_bytes())
            .map_err(|e| CorivoError::Internal(format!("auto-persona.tmp write failed: {e}")))?;
        f.sync_all()
            .map_err(|e| CorivoError::Internal(format!("auto-persona.tmp sync failed: {e}")))?;
    }
    std::fs::rename(&tmp_path, &final_path)
        .map_err(|e| CorivoError::Internal(format!("auto-persona.rename failed: {e}")))?;
    Ok(())
}

pub fn read(app_data_dir: &Path) -> Option<String> {
    let path = app_data_dir.join(AUTO_PERSONA_FILE);
    if !path.exists() {
        return None;
    }
    std::fs::read_to_string(&path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn write_then_read_roundtrip() {
        let dir = TempDir::new().unwrap();
        atomic_write(dir.path(), "# hello").unwrap();
        assert_eq!(read(dir.path()).as_deref(), Some("# hello"));
    }

    #[test]
    fn second_write_replaces_first() {
        let dir = TempDir::new().unwrap();
        atomic_write(dir.path(), "first").unwrap();
        atomic_write(dir.path(), "second").unwrap();
        assert_eq!(read(dir.path()).as_deref(), Some("second"));
    }
}
