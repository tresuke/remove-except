use std::path::{Path, PathBuf};

/// The type of an item found during traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link itself. Its target is not traversed.
    Symlink,
}

/// A filesystem item in the keep or deletion plan, captured at scan time.
///
/// This value is not automatically updated if the filesystem changes after planning.
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
    /// On Windows, the canonicalized path may include the extended-path prefix `\\?\`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the path relative to the root, using `/` separators.
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    /// Returns the type of this item.
    pub fn item_type(&self) -> ItemType {
        self.item_type
    }
}

/// A plan containing scan-time information and items to keep or delete beneath a root.
///
/// Creating this value or reading it through getters does not modify the filesystem.
/// Its contents are not automatically updated or revalidated if the filesystem changes
/// after the plan is created.
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
    /// Returns the normalized traversal root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the number of items directly matched by keep patterns; ancestors are not counted.
    pub fn direct_match_count(&self) -> usize {
        self.direct_match_count
    }

    /// Returns kept items and their ancestors, sorted by relative path.
    pub fn keep_items(&self) -> &[PlannedItem] {
        &self.keep_items
    }

    /// Returns all deletion targets, sorted by relative path.
    pub fn delete_items(&self) -> &[PlannedItem] {
        &self.delete_items
    }

    /// Returns the top-level items used for deletion.
    ///
    /// These items are candidates in the removal plan; this getter does not perform deletion.
    pub fn delete_roots(&self) -> &[PlannedItem] {
        &self.delete_roots
    }
}
