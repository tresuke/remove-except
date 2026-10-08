//! Creates a plan that keeps matching items and removes all others.
//!
//! [`build_plan`] scans beneath the root without including the root itself.
//! Multiple keep patterns may be provided; matching items and their ancestors are kept.
//! Plan creation does not change the filesystem. The returned plan is a snapshot from the
//! time of the scan; it does not reflect later filesystem changes or perform deletion.
//! Case matching uses Unicode simple case folding (non-Turkic) on Windows and is
//! case-sensitive on Ubuntu. Per-directory NTFS case-sensitivity settings are ignored.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use walkdir::WalkDir;

mod model;
mod path;
mod pattern;

pub use model::{ItemType, PlannedItem, RemovalPlan};
use path::{display_path, is_drive_relative_path, is_filesystem_root};
use pattern::compile_patterns;

/// Scans beneath the root and creates a plan of items to keep and delete.
///
/// Patterns specify items to keep. With multiple patterns, an item is kept if it matches
/// any pattern, along with its ancestors. `*` and `?` do not cross path separators, while
/// `**` matches across multiple levels. A literal directory path keeps its contents recursively.
/// Leading and trailing whitespace in patterns is preserved. Relative patterns are resolved
/// from the root; `.` components and repeated separators are normalized. Patterns containing
/// `..` are rejected. Relative roots are resolved from the process current directory.
///
/// Symbolic links are not followed and are treated as individual items. The root directory
/// itself is not included in the keep or delete lists. Plan creation does not modify files;
/// the result is a snapshot from the time of the scan. Filesystem changes after planning are
/// not automatically revalidated.
///
/// # Errors
///
/// Returns an error if no pattern is provided, a pattern is empty or has an invalid glob,
/// a pattern traverses to a parent directory, or an absolute pattern is outside the root.
/// An error is also returned if the root is a filesystem root or is not a directory, or if
/// resolving or scanning the root fails. Exact error messages are not a stable API contract.
///
/// # Examples
///
/// ```no_run
/// use remove_except::build_plan;
/// use std::path::Path;
///
/// let plan = build_plan(Path::new("work"), &["*.md".to_owned()])
///     .expect("the root and pattern should be valid");
/// for item in plan.delete_roots() {
///     println!("{}", item.relative_path());
/// }
/// ```
pub fn build_plan(root: &Path, patterns: &[String]) -> Result<RemovalPlan> {
    if patterns.is_empty() {
        bail!("At least one pattern is required");
    }
    if is_drive_relative_path(&root.to_string_lossy()) {
        bail!(
            "Drive-relative root paths are not supported: {}",
            root.display()
        );
    }

    let root = std::fs::canonicalize(root)
        .with_context(|| format!("Failed to resolve root directory: {}", root.display()))?;
    if is_filesystem_root(&root) {
        bail!(
            "Filesystem roots cannot be used as processing roots: {}",
            root.display()
        );
    }
    if !root.is_dir() {
        bail!("Root path is not a directory: {}", root.display());
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
        let keep = temp.path().join("keep.txt").to_string_lossy().into_owned();

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
