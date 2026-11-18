use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result, bail};

#[cfg(windows)]
fn reject_reparse_point(path: &Path, attributes: u32) -> Result<()> {
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!(
            "Cannot read metadata from a reparse point: {}",
            path.display()
        );
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct FileMetadata {
    pub(crate) size: u64,
    pub(crate) modified: Option<SystemTime>,
}

pub(crate) fn file_metadata(path: &Path, read_modified: bool) -> Result<FileMetadata> {
    #[cfg(windows)]
    let metadata = {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, SECURITY_ANONYMOUS,
            SECURITY_SQOS_PRESENT,
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
        reject_reparse_point(path, metadata.file_attributes())?;
        metadata
    };

    #[cfg(not(windows))]
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("Failed to read metadata for {}", path.display()))?;

    if !metadata.file_type().is_file() {
        bail!("Path is no longer a regular file: {}", path.display());
    }

    let modified =
        if read_modified {
            Some(metadata.modified().with_context(|| {
                format!("Failed to read modification time for {}", path.display())
            })?)
        } else {
            None
        };

    Ok(FileMetadata {
        size: metadata.len(),
        modified,
    })
}

#[cfg(test)]
mod tests {
    use super::file_metadata;
    use assert_fs::prelude::*;

    #[test]
    fn reads_size_without_requesting_modification_time() {
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file.txt");
        file.write_str("bytes").unwrap();

        let metadata = file_metadata(file.path(), false).unwrap();
        assert_eq!(metadata.size, 5);
        assert_eq!(metadata.modified, None);
    }

    #[test]
    fn reads_size_and_modification_time_from_the_same_metadata() {
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file.txt");
        file.write_str("bytes").unwrap();

        let metadata = file_metadata(file.path(), true).unwrap();
        assert_eq!(metadata.size, 5);
        assert!(metadata.modified.is_some());
    }

    #[test]
    fn rejects_directories() {
        let temp = assert_fs::TempDir::new().unwrap();
        let error = file_metadata(temp.path(), false).unwrap_err();
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
        let error = file_metadata(&link, true).unwrap_err();
        #[cfg(unix)]
        assert!(error.to_string().contains("no longer a regular file"));
        #[cfg(windows)]
        assert!(
            error
                .to_string()
                .contains("Cannot read metadata from a reparse point")
        );
        target.assert("target");
    }

    #[cfg(windows)]
    #[test]
    fn rejects_reparse_metadata_independently_of_link_type() {
        use super::reject_reparse_point;
        use std::path::Path;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_REPARSE_POINT,
        };

        assert!(reject_reparse_point(Path::new("placeholder"), FILE_ATTRIBUTE_ARCHIVE).is_ok());
        let error = reject_reparse_point(
            Path::new("placeholder"),
            FILE_ATTRIBUTE_ARCHIVE | FILE_ATTRIBUTE_REPARSE_POINT,
        )
        .unwrap_err();
        assert!(error.to_string().contains("reparse point"));
    }
}
