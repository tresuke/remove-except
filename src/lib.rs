#![warn(missing_docs)]

//! Safe removal plan generation for cleaning directories while keeping specified items.
//!
//! `remove-except` scans beneath a chosen processing root, compares discovered filesystem
//! items against one or more keep patterns, and creates an in-memory [`RemovalPlan`].
//! The plan partitions items into kept entries (including required ancestor directories)
//! and deletion targets.
//!
//! # Safety by Design
//!
//! - **Separation of Planning and Deletion**: [`build_plan`] is strictly read-only.
//!   It records an in-memory plan from entries observed during traversal. The traversal is not
//!   an atomic or protected filesystem snapshot and does not lock or prevent concurrent changes.
//!   It does not create, modify, or delete any files or directories.
//! - **Root Protection**: The processing root itself is never included in the plan,
//!   and filesystem roots (such as `/` on Unix or `C:\` on Windows) are rejected to prevent
//!   catastrophic deletion.
//! - **No Symlink Traversal**: Symbolic links encountered beneath the root are never followed.
//!   Each link is evaluated as an individual leaf item; if targeted for deletion, only the
//!   symbolic link itself is deleted, never its target.
//!
//! # Pattern Matching Syntax and Semantics
//!
//! At least one keep pattern must be provided. When multiple patterns are given, an item
//! is kept if it matches **any** pattern (logical OR):
//!
//! - `*` matches any sequence of non-separator characters within a single directory component.
//! - `?` matches any single non-separator character.
//! - `**` matches zero or more directory levels.
//! - `[...]` matches any character in the character class, adhering to standard glob rules.
//! - `{a,b}` matches any of the comma-separated alternatives.
//! - `.` or `./` represents the processing root and keeps everything beneath it.
//! - A directory path without glob syntax keeps that directory and all of its contents
//!   recursively. Glob syntax is `*`, `?`, `[`, `{`, and `}`; on non-Windows platforms,
//!   `\` is also glob syntax because it escapes the next character. On Windows, `\` is a
//!   path separator.
//! - A glob match keeps the matching item and all required ancestor directories up to the root,
//!   but does *not* recursively keep unmatched descendants. For instance, `dir/*` matches
//!   items directly beneath `dir/`, but does not recursively keep unmatched contents of
//!   subdirectories beneath them. Likewise, `dir/**` does not match `dir` itself, and an
//!   escaped name such as `backup[[]2025]` keeps only that directory; append `/**` to keep
//!   its contents.
//! - Relative patterns are resolved relative to the processing root. Patterns containing a
//!   `..` component are rejected.
//! - Leading and trailing whitespace in patterns is preserved.
//!
//! # Platform Differences
//!
//! - **Windows**: Path matching uses Unicode simple case folding (non-Turkic), ignoring
//!   per-directory NTFS case-sensitivity settings.
//! - **Ubuntu**: Path matching is strictly case-sensitive. Windows and Ubuntu are the
//!   supported platforms.
//!
//! # Examples
//!
//! ```
//! use remove_except::build_plan;
//! use std::fs::{self, File};
//!
//! let temp_dir = assert_fs::TempDir::new().unwrap();
//! let root = temp_dir.path();
//! fs::create_dir_all(root.join("src")).unwrap();
//! File::create(root.join("src/main.rs")).unwrap();
//! File::create(root.join("notes.txt")).unwrap();
//!
//! // Build a removal plan keeping only files inside `src/`:
//! let plan = build_plan(root, &["src/**".to_owned()])
//!     .expect("valid root and keep patterns");
//!
//! // The plan identifies kept items and ancestors:
//! assert!(plan.keep_items().iter().any(|item| item.relative_path() == "src/main.rs"));
//! assert!(plan.keep_items().iter().any(|item| item.relative_path() == "src"));
//!
//! // Deletion roots identify top-level removal targets:
//! assert!(plan.delete_roots().iter().any(|item| item.relative_path() == "notes.txt"));
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

mod error;
mod model;
mod path;
mod pattern;

pub use error::ValidationError;
use error::{invalid, invalid_with_source};
pub use model::{ItemType, PlannedItem, RemovalPlan};
use path::{display_path, is_drive_relative_path, is_filesystem_root};
use pattern::compile_patterns;

/// Scans beneath the specified root directory and constructs a [`RemovalPlan`].
///
/// Discovered filesystem entries are evaluated against the provided `patterns`. An item is
/// kept if it matches any pattern (logical OR). When an item is kept, all of its ancestor
/// directories up to `root` are also kept to preserve directory hierarchy.
///
/// # Arguments
///
/// * `root` - Path to the directory to scan. Relative paths are resolved against the current
///   working directory. Filesystem roots (e.g. `/` on Unix, `C:\` on Windows) and Windows
///   drive-relative paths (e.g. `C:foo`) are rejected.
/// * `patterns` - One or more keep patterns. Patterns may be literal relative paths or glob
///   patterns (`*`, `?`, `**`, `[...]`, `{a,b}`; on non-Windows platforms `\` escapes the
///   next character). Literal directories without glob syntax match recursively; glob
///   matches do not keep unmatched descendants. Relative patterns are resolved relative to
///   `root`. Patterns containing a `..` component are rejected. Leading and trailing
///   whitespace is preserved.
///
/// # Safety and Behavior
///
/// - **Read-Only**: This function only traverses and reads directory metadata. It does not
///   modify, create, or delete any files.
/// - **No Symlink Traversal**: Symbolic links beneath `root` are treated as individual leaf
///   items and are never followed.
/// - **Root Exclusion**: The `root` directory itself is never included in the plan.
/// - **Observed Entries**: The plan contains entries observed during traversal. The traversal
///   is not atomic and does not lock or prevent concurrent filesystem changes. The plan is not
///   refreshed if the filesystem changes after construction.
///
/// # Errors
///
/// Returns an error in the following circumstances (exact error message strings are not part
/// of the stable API contract):
///
/// - **Pattern errors**: `patterns` is empty, a pattern contains invalid glob syntax, or a
///   pattern escapes `root` (e.g., starts with `../` or resolves to an ancestor).
/// - **Root errors**: `root` cannot be resolved, is not a directory, is a filesystem root,
///   or is a Windows drive-relative path.
/// - **I/O errors**: Directory traversal fails due to filesystem permissions or I/O errors.
///
/// Pattern and root errors carry a [`ValidationError`] that can be detected with
/// [`anyhow::Error::downcast_ref`]. Traversal errors do not.
///
/// # Examples
///
/// ```
/// use remove_except::build_plan;
/// use std::fs::{self, File};
///
/// let temp_dir = assert_fs::TempDir::new().unwrap();
/// let root = temp_dir.path();
/// fs::create_dir_all(root.join("docs")).unwrap();
/// File::create(root.join("docs/guide.md")).unwrap();
/// File::create(root.join("temp.log")).unwrap();
///
/// let plan = build_plan(root, &["docs/**".to_owned()])
///     .expect("plan construction should succeed");
///
/// assert_eq!(plan.direct_match_count(), 1);
/// assert!(plan.keep_items().iter().any(|item| item.relative_path() == "docs/guide.md"));
/// assert!(plan.delete_roots().iter().any(|item| item.relative_path() == "temp.log"));
/// ```
pub fn build_plan(root: &Path, patterns: &[String]) -> Result<RemovalPlan> {
    if patterns.is_empty() {
        return Err(invalid("At least one pattern is required"));
    }
    if is_drive_relative_path(&root.to_string_lossy()) {
        return Err(invalid(format!(
            "Drive-relative root paths are not supported: {}",
            root.display()
        )));
    }

    let root = std::fs::canonicalize(root).map_err(|error| {
        invalid_with_source(
            error,
            format!("Failed to resolve root directory: {}", root.display()),
        )
    })?;
    if is_filesystem_root(&root) {
        return Err(invalid(format!(
            "Filesystem roots cannot be used as processing roots: {}",
            root.display()
        )));
    }
    if !root.is_dir() {
        return Err(invalid(format!(
            "Root path is not a directory: {}",
            root.display()
        )));
    }

    let compiled_patterns = compile_patterns(&root, patterns)?;
    let mut entries = Vec::new();
    // WalkDir yields entries depth-first, so retain indices of the current item's ancestors.
    let mut directory_stack = Vec::new();
    let mut direct_match_count = 0;

    for entry in WalkDir::new(&root).follow_links(false).min_depth(1) {
        let entry = entry.with_context(|| "Failed to walk root directory")?;
        let depth = entry.depth();
        directory_stack.truncate(depth.saturating_sub(1));
        // Parents are visited before children, so a parent's index is always smaller.
        let parent_index = directory_stack.last().copied();
        let file_type = entry.file_type();
        let path = entry.path().to_path_buf();
        let relative = path
            .strip_prefix(&root)
            .context("Walked path escaped the root directory")?;
        let relative_path = display_path(relative);
        let matched = compiled_patterns.matches(&relative_path);

        if matched {
            direct_match_count += 1;
        }

        let item_type = if file_type.is_symlink() {
            ItemType::Symlink
        } else if file_type.is_dir() {
            ItemType::Directory
        } else {
            ItemType::File
        };
        let item_index = entries.len();
        entries.push(IndexedItem {
            item: PlannedItem {
                path,
                relative_path,
                item_type,
            },
            parent_index,
            keep: matched,
            is_delete_root: false,
        });
        if item_type == ItemType::Directory {
            directory_stack.push(item_index);
        }
    }

    Ok(finalize_plan(root, direct_match_count, entries))
}

/// Derives kept ancestors and top-level deletion items from recorded parent indices.
fn finalize_plan(
    root: PathBuf,
    direct_match_count: usize,
    mut entries: Vec<IndexedItem>,
) -> RemovalPlan {
    // Children follow parents, so walk in reverse to propagate keep state to ancestors.
    for index in (0..entries.len()).rev() {
        if entries[index].keep
            && let Some(parent_index) = entries[index].parent_index
        {
            entries[parent_index].keep = true;
        }
    }

    // An unkept item is a deletion root when its parent is kept or is the processing root.
    for index in 0..entries.len() {
        let parent_is_kept = entries[index]
            .parent_index
            .is_none_or(|parent_index| entries[parent_index].keep);
        entries[index].is_delete_root = !entries[index].keep && parent_is_kept;
    }

    entries.sort_unstable_by(|left, right| left.item.relative_path.cmp(&right.item.relative_path));
    let mut keep_items = Vec::new();
    let mut delete_items = Vec::new();
    let mut delete_roots = Vec::new();
    for entry in entries {
        if entry.keep {
            keep_items.push(entry.item);
        } else {
            if entry.is_delete_root {
                delete_roots.push(entry.item.clone());
            }
            delete_items.push(entry.item);
        }
    }

    RemovalPlan {
        root,
        direct_match_count,
        keep_items,
        delete_items,
        delete_roots,
    }
}

/// Data used to derive kept ancestors and top-level deletion items from traversal order.
struct IndexedItem {
    /// The item included in the public plan.
    item: PlannedItem,
    /// Index of the parent directory in traversal order, unless directly under the root.
    parent_index: Option<usize>,
    /// Initialized from direct matches and propagated to their ancestors.
    keep: bool,
    /// True when this is a top-level item passed to the deletion routine.
    is_delete_root: bool,
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::path::is_drive_relative_path;
    #[cfg(windows)]
    use super::pattern::normalize_pattern;
    use super::{ItemType, build_plan, is_filesystem_root};
    use assert_fs::prelude::*;
    use std::path::Path;

    #[test]
    // Verify filesystem roots are identified correctly on Unix and Windows.
    fn identifies_filesystem_root_paths() {
        assert!(is_filesystem_root(Path::new("/")));

        #[cfg(windows)]
        {
            assert!(is_filesystem_root(Path::new("C:\\")));
            assert!(is_filesystem_root(Path::new("\\\\server\\share\\")));
            assert!(!is_filesystem_root(Path::new("C:\\folder")));
        }
    }

    #[cfg(windows)]
    #[test]
    // Verify Windows drive-relative paths are rejected as both roots and keep patterns.
    fn rejects_drive_relative_roots_and_patterns() {
        assert!(is_drive_relative_path("C:folder"));
        assert!(!is_drive_relative_path("C:\\folder"));
        assert!(!is_drive_relative_path("\\\\server\\share\\folder"));

        let root_error = build_plan(Path::new("C:folder"), &["*".to_owned()]).unwrap_err();
        assert!(root_error.to_string().contains("Drive-relative root"));

        let pattern_error = normalize_pattern("C:/root", "C:folder").unwrap_err();
        assert!(pattern_error.to_string().contains("Drive-relative pattern"));
    }

    #[test]
    // Verify a filesystem root is rejected before scanning begins.
    fn rejects_filesystem_root_before_walking_it() {
        let error = build_plan(Path::new("/"), &["*".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("Filesystem roots"));
    }

    #[test]
    // Verify missing patterns, empty patterns, and invalid globs are rejected.
    fn rejects_empty_and_invalid_patterns() {
        let temp = assert_fs::TempDir::new().unwrap();

        let missing_error = build_plan(temp.path(), &[]).unwrap_err();
        assert!(missing_error.to_string().contains("At least one pattern"));

        let empty_error = build_plan(temp.path(), &[String::new()]).unwrap_err();
        assert!(
            empty_error
                .to_string()
                .contains("Pattern must not be empty")
        );

        let invalid_error = build_plan(temp.path(), &["[".to_owned()]).unwrap_err();
        assert!(invalid_error.to_string().contains("Invalid glob pattern"));
    }

    #[test]
    // Verify a non-directory path is rejected as the processing root.
    fn rejects_non_directory_root() {
        let temp = assert_fs::TempDir::new().unwrap();
        let root_file = temp.child("root.txt");
        root_file.write_str("not a directory").unwrap();

        let error = build_plan(root_file.path(), &["*".to_owned()]).unwrap_err();

        assert!(error.to_string().contains("Root path is not a directory"));
    }

    #[test]
    // Verify matched items and ancestors are kept and all other items are planned for deletion.
    fn keeps_matching_item_and_its_ancestors() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("keep/nested/file.txt")
            .write_str("keep")
            .unwrap();
        temp.child("remove.txt").write_str("remove").unwrap();

        let plan = build_plan(temp.path(), &["keep/nested/file.txt".to_owned()]).unwrap();

        assert_eq!(plan.direct_match_count, 1);
        assert_eq!(plan.keep_items.len(), 3);
        assert!(
            plan.keep_items
                .iter()
                .any(|item| item.relative_path == "keep/nested/file.txt")
        );
        assert_eq!(plan.delete_items.len(), 1);
        assert_eq!(plan.delete_items[0].relative_path, "remove.txt");
        assert_eq!(plan.delete_items[0].item_type, ItemType::File);
        assert_eq!(plan.delete_roots.len(), 1);
    }

    #[test]
    // Verify a literal directory pattern keeps the entire subtree.
    fn directory_prefix_keeps_its_contents() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("keep/deep/file.txt").write_str("keep").unwrap();
        temp.child("remove/file.txt").write_str("remove").unwrap();

        let plan = build_plan(temp.path(), &["keep".to_owned()]).unwrap();

        assert_eq!(plan.direct_match_count, 3);
        assert_eq!(plan.keep_items.len(), 3);
        assert!(
            plan.keep_items
                .iter()
                .any(|item| item.relative_path == "keep/deep/file.txt")
        );
        assert_eq!(plan.delete_items.len(), 2);
        assert_eq!(plan.delete_roots.len(), 1);
        assert_eq!(plan.delete_roots[0].relative_path, "remove");
    }

    #[test]
    // Verify `*` matches one level and `**` matches paths at multiple levels.
    fn globstar_matches_nested_paths_while_star_matches_one_level() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("root.md").write_str("keep").unwrap();
        temp.child("nested/child.md").write_str("keep").unwrap();
        temp.child("nested/deep/annual.md")
            .write_str("keep")
            .unwrap();

        let single_level = build_plan(temp.path(), &["*.md".to_owned()]).unwrap();
        assert_eq!(single_level.direct_match_count, 1);
        assert!(
            single_level
                .delete_items
                .iter()
                .any(|item| item.relative_path == "nested/child.md")
        );

        let all_levels = build_plan(temp.path(), &["**/*.md".to_owned()]).unwrap();
        assert_eq!(all_levels.direct_match_count, 3);
        assert!(all_levels.delete_items.is_empty());
    }

    #[test]
    // Verify `?` and character classes each match one character.
    fn question_mark_and_character_class_match_one_character() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("file1.c").write_str("keep").unwrap();
        temp.child("file2.h").write_str("keep").unwrap();
        temp.child("file12.c").write_str("remove").unwrap();
        temp.child("file3.txt").write_str("remove").unwrap();

        let plan = build_plan(temp.path(), &["file?.[ch]".to_owned()]).unwrap();

        assert_eq!(plan.direct_match_count, 2);
        assert_eq!(plan.delete_items.len(), 2);
        assert!(
            plan.delete_items
                .iter()
                .any(|item| item.relative_path == "file12.c")
        );
        assert!(
            plan.delete_items
                .iter()
                .any(|item| item.relative_path == "file3.txt")
        );
    }

    #[test]
    // Verify brace alternation is glob syntax, so it does not keep matched directory contents.
    fn brace_alternation_is_glob_syntax_and_does_not_keep_contents() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("reports/2024/a.txt").write_str("a").unwrap();
        temp.child("reports/2025/b.txt").write_str("b").unwrap();
        temp.child("reports/2026/c.txt").write_str("c").unwrap();
        temp.child("x{1}.txt").write_str("brace").unwrap();
        temp.child("x1.txt").write_str("plain").unwrap();

        let dirs_only = build_plan(temp.path(), &["reports/{2024,2025}".to_owned()]).unwrap();
        let kept = |plan: &super::RemovalPlan| {
            plan.keep_items
                .iter()
                .map(|item| item.relative_path.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kept(&dirs_only),
            ["reports", "reports/2024", "reports/2025"]
        );

        let with_contents =
            build_plan(temp.path(), &["reports/{2024,2025}/**".to_owned()]).unwrap();
        assert_eq!(
            kept(&with_contents),
            [
                "reports",
                "reports/2024",
                "reports/2024/a.txt",
                "reports/2025",
                "reports/2025/b.txt"
            ]
        );

        // `{1}` is a one-choice alternation, so it matches `x1.txt`, not the literal name.
        let literal_braces = build_plan(temp.path(), &["x{1}.txt".to_owned()]).unwrap();
        assert_eq!(kept(&literal_braces), ["x1.txt"]);
        let escaped_braces = build_plan(temp.path(), &["x[{]1[}].txt".to_owned()]).unwrap();
        assert_eq!(kept(&escaped_braces), ["x{1}.txt"]);
    }

    #[test]
    // Verify an escaped bracket directory name keeps only the directory unless `/**` follows.
    fn escaped_bracket_directory_needs_globstar_to_keep_contents() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("backup[2025]/a.txt").write_str("a").unwrap();

        let plan = build_plan(temp.path(), &["backup[[]2025]".to_owned()]).unwrap();
        assert_eq!(plan.keep_items.len(), 1);
        assert_eq!(plan.delete_items[0].relative_path, "backup[2025]/a.txt");

        let plan = build_plan(temp.path(), &["backup[[]2025]/**".to_owned()]).unwrap();
        assert!(plan.delete_items.is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    // Verify a backslash is a glob escape on non-Windows platforms, not a literal prefix.
    fn backslash_is_glob_escape_on_non_windows() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("subfile.txt").write_str("escaped").unwrap();
        temp.child("sub\\file.txt/inner.txt")
            .write_str("literal directory")
            .unwrap();

        let plan = build_plan(temp.path(), &["sub\\file.txt".to_owned()]).unwrap();
        assert_eq!(plan.keep_items.len(), 1);
        assert_eq!(plan.keep_items[0].relative_path, "subfile.txt");
    }

    #[test]
    // Verify `dir/*` matches direct children but does not keep nested descendants.
    fn single_level_glob_does_not_keep_nested_directory_contents() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("dir/top.txt").write_str("keep").unwrap();
        temp.child("dir/sub/nested.txt")
            .write_str("remove")
            .unwrap();

        let plan = build_plan(temp.path(), &["dir/*".to_owned()]).unwrap();

        assert_eq!(plan.direct_match_count, 2);
        assert_eq!(plan.delete_items.len(), 1);
        assert_eq!(plan.delete_items[0].relative_path, "dir/sub/nested.txt");
    }

    #[test]
    // Verify keep patterns that traverse to a parent directory are rejected.
    fn rejects_parent_traversal() {
        let temp = assert_fs::TempDir::new().unwrap();
        let error = build_plan(temp.path(), &["../outside".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("Parent directory traversal"));
    }

    #[test]
    // Verify absolute keep patterns outside the root are rejected.
    fn absolute_pattern_must_stay_within_root_boundary() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("keep.txt").write_str("keep").unwrap();
        let sibling = temp.path().with_file_name(format!(
            "{}-sibling",
            temp.path().file_name().unwrap().to_string_lossy()
        ));
        let outside_pattern = sibling.join("*").to_string_lossy().into_owned();

        let error = build_plan(temp.path(), &[outside_pattern]).unwrap_err();

        assert!(error.to_string().contains("outside the processing root"));
    }

    #[test]
    // Verify an absolute keep pattern inside the root keeps the matching item.
    fn absolute_keep_pattern_matches_inside_root() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("keep.txt").write_str("keep").unwrap();
        temp.child("remove.txt").write_str("remove").unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let keep = root.join("keep.txt").to_string_lossy().into_owned();

        let plan = build_plan(temp.path(), &[keep]).unwrap();

        assert_eq!(plan.delete_items.len(), 1);
        assert_eq!(plan.delete_items[0].relative_path, "remove.txt");
    }

    #[test]
    // Verify glob case matching follows the platform-specific policy.
    fn glob_case_sensitivity_matches_platform() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("Keep.txt").write_str("keep").unwrap();

        let plan = build_plan(temp.path(), &["keep.txt".to_owned()]).unwrap();

        assert_eq!(plan.delete_items.is_empty(), cfg!(windows));
    }

    #[cfg(windows)]
    #[test]
    // Verify Windows root-relative patterns outside the selected root are rejected.
    fn rejects_windows_root_relative_pattern_outside_current_root() {
        let temp = assert_fs::TempDir::new().unwrap();

        let error = build_plan(temp.path(), &["/outside/*".to_owned()]).unwrap_err();

        assert!(error.to_string().contains("outside the processing root"));
    }
}
