use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result, bail};

pub(crate) fn modified_time(path: &Path) -> Result<SystemTime> {
    #[cfg(windows)]
    let metadata = {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            SECURITY_ANONYMOUS, SECURITY_SQOS_PRESENT,
        };

        let file = std::fs::OpenOptions::new()
            .access_mode(0)
            .custom_flags(
                FILE_FLAG_OPEN_REPARSE_POINT
                    | FILE_FLAG_BACKUP_SEMANTICS
                    | SECURITY_SQOS_PRESENT
                    | SECURITY_ANONYMOUS,
            )
            .open(path)
            .with_context(|| format!("Failed to open metadata for {}", path.display()))?;
        let metadata = file.metadata().context("Failed to read file metadata")?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            bail!(
                "Cannot read modification time from a reparse point: {}",
                path.display()
            );
        }
        metadata
    };

    #[cfg(not(windows))]
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("Failed to read metadata for {}", path.display()))?;

    if !metadata.file_type().is_file() {
        bail!("File is no longer a regular file: {}", path.display());
    }
    metadata
        .modified()
        .with_context(|| format!("Failed to read modification time for {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::modified_time;
    use assert_fs::prelude::*;

    #[test]
    fn rejects_directories() {
        let temp = assert_fs::TempDir::new().unwrap();
        let error = modified_time(temp.path()).unwrap_err();
        assert!(error.to_string().contains("no longer a regular file"));
    }

    #[test]
    fn rejects_symlinks_without_following_them() {
        let temp = assert_fs::TempDir::new().unwrap();
        let target = temp.child("target.txt");
        target.write_str("target").unwrap();
        let link = temp.path().join("link");
        #[cfg(unix)]
        let link_result = std::os::unix::fs::symlink(target.path(), &link);
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_file(target.path(), &link);
        link_result.expect("creating the symbolic-link fixture is required for this test");
        let error = modified_time(&link).unwrap_err();
        #[cfg(unix)]
        assert!(error.to_string().contains("no longer a regular file"));
        #[cfg(windows)]
        assert!(
            error
                .to_string()
                .contains("Cannot read modification time from a reparse point")
        );
        target.assert("target");
    }
}
