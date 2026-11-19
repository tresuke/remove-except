use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;

use crate::error::invalid;

/// A half-open range of modification instants.
///
/// The lower bound is inclusive and the upper bound is exclusive. Bounds retain the
/// precision and range of the supplied `SystemTime` values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifiedRange {
    since: Option<SystemTime>,
    before: Option<SystemTime>,
}

impl ModifiedRange {
    /// Constructs a range with at least one bound.
    ///
    /// Returns a [`crate::ValidationError`] if both bounds are absent or the lower
    /// bound is greater than or equal to the upper bound.
    pub fn new(since: Option<SystemTime>, before: Option<SystemTime>) -> Result<Self> {
        if since.is_none() && before.is_none() {
            return Err(invalid("At least one modification-time bound is required"));
        }
        if let (Some(since), Some(before)) = (since, before)
            && since >= before
        {
            return Err(invalid("Modification-time start must precede the end"));
        }
        Ok(Self { since, before })
    }

    /// Returns the inclusive lower bound, if specified.
    pub fn since(&self) -> Option<SystemTime> {
        self.since
    }

    /// Returns the exclusive upper bound, if specified.
    pub fn before(&self) -> Option<SystemTime> {
        self.before
    }

    pub(crate) fn contains(&self, time: SystemTime) -> bool {
        self.since.is_none_or(|since| time >= since)
            && self.before.is_none_or(|before| time < before)
    }
}

/// A half-open range of file sizes in bytes.
///
/// The lower bound is inclusive and the upper bound is exclusive. File sizes are
/// logical lengths reported by filesystem metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeRange {
    at_least: Option<u64>,
    below: Option<u64>,
}

impl SizeRange {
    /// Constructs a size range with at least one bound.
    ///
    /// Returns a [`crate::ValidationError`] if both bounds are absent, the exclusive
    /// upper bound is zero, or the lower bound is greater than or equal to the upper bound.
    pub fn new(at_least: Option<u64>, below: Option<u64>) -> Result<Self> {
        if at_least.is_none() && below.is_none() {
            return Err(invalid("At least one file-size bound is required"));
        }
        if below == Some(0) {
            return Err(invalid("File-size upper bound must be greater than zero"));
        }
        if let (Some(at_least), Some(below)) = (at_least, below)
            && at_least >= below
        {
            return Err(invalid(
                "File-size lower bound must be less than the upper bound",
            ));
        }
        Ok(Self { at_least, below })
    }

    /// Returns the inclusive lower bound in bytes, if specified.
    pub fn at_least(&self) -> Option<u64> {
        self.at_least
    }

    /// Returns the exclusive upper bound in bytes, if specified.
    pub fn below(&self) -> Option<u64> {
        self.below
    }

    pub(crate) fn contains(&self, size: u64) -> bool {
        self.at_least.is_none_or(|at_least| size >= at_least)
            && self.below.is_none_or(|below| size < below)
    }
}

/// Optional conditions used while generating a removal plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanOptions {
    modified_range: Option<ModifiedRange>,
    size_range: Option<SizeRange>,
}

impl PlanOptions {
    /// Constructs options with a modification-time range.
    pub fn new(modified_range: ModifiedRange) -> Self {
        Self {
            modified_range: Some(modified_range),
            size_range: None,
        }
    }

    /// Adds a file-size range to these options.
    ///
    /// This can be used with [`PlanOptions::default`] for size-only selection or chained
    /// after [`PlanOptions::new`] to combine size and modification-time conditions.
    pub fn with_size_range(mut self, size_range: SizeRange) -> Self {
        self.size_range = Some(size_range);
        self
    }

    /// Returns the modification-time range, if specified.
    pub fn modified_range(&self) -> Option<&ModifiedRange> {
        self.modified_range.as_ref()
    }

    /// Returns the file-size range, if specified.
    pub fn size_range(&self) -> Option<&SizeRange> {
        self.size_range.as_ref()
    }
}

/// The type of a filesystem item found during traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    /// An item other than a directory or symbolic link, including special files.
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
/// Created by [`crate::build_plan`] or [`crate::build_plan_with_options`]. Constructing or querying a `RemovalPlan` is strictly
/// read-only and never modifies the filesystem.
///
/// # Item Collections
///
/// A `RemovalPlan` separates discovered filesystem items into three collections:
///
/// - [`keep_items`](RemovalPlan::keep_items): Contains all items directly matched by keep
///   conditions, plus all ancestor directories required to preserve their paths from the root.
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
    /// Number of items directly matching all keep conditions; ancestors alone are not counted.
    pub(crate) direct_match_count: usize,
    pub(crate) modified_range: Option<ModifiedRange>,
    pub(crate) size_range: Option<SizeRange>,
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

    /// Returns the number of items that directly matched all keep conditions.
    ///
    /// Directories kept solely because they are ancestors of matched items are not
    /// included in this count.
    pub fn direct_match_count(&self) -> usize {
        self.direct_match_count
    }

    /// Returns the modification-time range used to create the plan, if specified.
    pub fn modified_range(&self) -> Option<&ModifiedRange> {
        self.modified_range.as_ref()
    }

    /// Returns the file-size range used to create the plan, if specified.
    pub fn size_range(&self) -> Option<&SizeRange> {
        self.size_range.as_ref()
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
