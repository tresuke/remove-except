use std::path::{Path, PathBuf};

/// The type of a filesystem item found during traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link. Symbolic links are evaluated as leaf items and never traversed.
    Symlink,
}

/// A filesystem item observed during the traversal that created a removal plan.
///
/// The traversal is not atomic. Instances are not automatically updated or revalidated if
/// filesystem contents change during or after planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedItem {
    /// Absolute path to the item, resolved under the normalized root.
    /// On Windows, the canonicalized path may include the extended-path prefix `\\?\`.
    pub(crate) path: PathBuf,
    /// Path relative to the root, using `/` separators.
    pub(crate) relative_path: String,
    /// The type of this item.
    pub(crate) item_type: ItemType,
}

impl PlannedItem {
    /// Returns the absolute path to the item, resolved under the normalized root.
    ///
    /// On Windows, the canonicalized path may include the extended-length path prefix (`\\?\`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the item's path relative to the processing root, always normalized with `/` separators.
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    /// Returns the [`ItemType`] classification of this item.
    pub fn item_type(&self) -> ItemType {
        self.item_type
    }
}

/// An in-memory plan containing items to keep or delete beneath a root directory.
///
/// Created by [`crate::build_plan`]. Constructing or querying a `RemovalPlan` is strictly
/// read-only and never modifies the filesystem.
///
/// # Item Collections
///
/// A `RemovalPlan` separates discovered filesystem items into three collections:
///
/// - [`keep_items`](RemovalPlan::keep_items): Contains all items directly matched by keep
///   patterns, plus all ancestor directories required to preserve their paths from the root.
/// - [`delete_items`](RemovalPlan::delete_items): Contains every individual item targeted for
///   deletion. This comprehensive list is suitable for full previews, logging, or reporting.
/// - [`delete_roots`](RemovalPlan::delete_roots): Contains only the top-level deletion targets.
///   When an entire directory is targeted for deletion, its descendants appear in `delete_items`
///   but are excluded from `delete_roots`. This minimal set is ideal for executing deletion via
///   recursive removal routines without redundant operations.
#[derive(Debug)]
pub struct RemovalPlan {
    /// The normalized traversal root. The root itself is not included in the plan.
    pub(crate) root: PathBuf,
    /// Number of items directly matched by keep patterns; ancestors alone are not counted.
    pub(crate) direct_match_count: usize,
    /// Items to keep and their ancestors, sorted by relative path.
    pub(crate) keep_items: Vec<PlannedItem>,
    /// All deletion targets, sorted by relative path.
    pub(crate) delete_items: Vec<PlannedItem>,
    /// Top-level items used for deletion. Descendants of deletion-target directories are excluded.
    pub(crate) delete_roots: Vec<PlannedItem>,
}

impl RemovalPlan {
    /// Returns the normalized root directory of the plan.
    ///
    /// The root directory itself is never included in any keep or delete list.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the number of items that directly matched one or more keep patterns.
    ///
    /// Directories kept solely because they are ancestors of matched items are not
    /// included in this count.
    pub fn direct_match_count(&self) -> usize {
        self.direct_match_count
    }

    /// Returns all kept items and their required ancestor directories, sorted by relative path.
    pub fn keep_items(&self) -> &[PlannedItem] {
        &self.keep_items
    }

    /// Returns all items targeted for deletion, sorted by relative path.
    ///
    /// This is the complete, exhaustive list of all deletion targets, including items inside
    /// directories that will be removed.
    pub fn delete_items(&self) -> &[PlannedItem] {
        &self.delete_items
    }

    /// Returns the minimal top-level items targeted for deletion, sorted by relative path.
    ///
    /// If a directory is targeted for deletion, the directory itself is included in this list,
    /// but its child items are omitted. This provides the minimal set of targets needed to
    /// execute recursive deletion safely and efficiently.
    pub fn delete_roots(&self) -> &[PlannedItem] {
        &self.delete_roots
    }
}
