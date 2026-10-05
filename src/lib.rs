//! パターンに一致した項目を保持し、それ以外を削除する計画を作成します。
//!
//! [`build_plan`] はルートディレクトリ自体を残したまま、その配下を走査します。
//! 保持パターンは複数指定でき、いずれかに一致した項目とその祖先を保持します。

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use walkdir::WalkDir;

/// 走査対象内で見つかった項目の種類です。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    /// 通常のファイルです。
    File,
    /// ディレクトリです。
    Directory,
    /// シンボリックリンク自体です。リンク先は走査しません。
    Symlink,
}

/// 保持または削除の計画に含まれるファイルシステム項目です。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedItem {
    /// ルートを基準に解決した項目の絶対パスです。
    pub path: PathBuf,
    /// ルートからの相対パスです。区切り文字には `/` を使います。
    pub relative_path: String,
    /// 項目の種類です。
    pub item_type: ItemType,
}

/// ルート配下の走査結果と保持・削除対象をまとめた計画です。
#[derive(Debug)]
pub struct RemovalPlan {
    /// 正規化された走査ルートです。このパス自体は計画に含まれません。
    pub root: PathBuf,
    /// 保持パターンに直接一致した項目数です。祖先のみの項目は含みません。
    pub direct_match_count: usize,
    /// 保持される項目とその祖先です。相対パス順に並びます。
    pub keep_items: Vec<PlannedItem>,
    /// 削除対象となる全項目です。相対パス順に並びます。
    pub delete_items: Vec<PlannedItem>,
    /// 実削除に使う最上位の項目です。削除対象ディレクトリ内の子孫は含みません。
    pub delete_roots: Vec<PlannedItem>,
}

struct PatternSpec {
    glob_index: usize,
    prefix: Option<String>,
}

/// ルート配下を走査し、保持・削除対象の計画を作成します。
///
/// パターンは保持対象を指定します。複数のパターンを指定した場合はいずれかに
/// 一致した項目を保持し、その祖先も保持します。`*` と `?` はパス区切りをまたがず、
/// `**` は複数階層に一致します。リテラルのディレクトリ指定は配下も再帰的に保持します。
///
/// シンボリックリンクはたどらず、リンク自体を1項目として扱います。ルートディレクトリ
/// 自体は保持・削除一覧に含まれません。
///
/// # Errors
///
/// パターンが空、無効なGlob、親ディレクトリへの移動を含む、またはルート外の絶対パスを
/// 指す場合にエラーを返します。ルートの解決や走査に失敗した場合もエラーになります。
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

    let (glob_set, pattern_specs) = compile_patterns(&root, patterns)?;
    let mut kept_paths = HashSet::new();
    let mut entries = Vec::new();
    let mut direct_match_count = 0;

    for entry in WalkDir::new(&root).follow_links(false).min_depth(1) {
        let entry = entry.with_context(|| "Failed to walk root directory")?;
        let path = entry.path().to_path_buf();
        let relative = path
            .strip_prefix(&root)
            .context("Walked path escaped the root directory")?;
        let relative_path = display_path(relative);
        let matched_globs = glob_set.matches(&relative_path);
        let matched = pattern_specs.iter().any(|spec| {
            matched_globs.contains(&spec.glob_index)
                || spec
                    .prefix
                    .as_ref()
                    .is_some_and(|prefix| path_matches_prefix(&relative_path, prefix))
        });

        if matched {
            direct_match_count += 1;
            let mut current = Some(path.as_path());
            while let Some(path) = current {
                if !path.starts_with(&root) {
                    break;
                }
                kept_paths.insert(path.to_path_buf());
                if path == root {
                    break;
                }
                current = path.parent();
            }
        }

        let file_type = entry.file_type();
        let item_type = if file_type.is_symlink() {
            ItemType::Symlink
        } else if file_type.is_dir() {
            ItemType::Directory
        } else {
            ItemType::File
        };
        entries.push(PlannedItem {
            path,
            relative_path,
            item_type,
        });
    }

    let (mut keep_items, mut delete_items): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|item| kept_paths.contains(&item.path));
    keep_items.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    delete_items.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let deleted_directories: HashSet<_> = delete_items
        .iter()
        .filter(|item| item.item_type == ItemType::Directory)
        .map(|item| item.path.as_path())
        .collect();
    let delete_roots = delete_items
        .iter()
        .filter(|item| {
            let mut parent = item.path.parent();
            while let Some(path) = parent {
                if path == root {
                    break;
                }
                if deleted_directories.contains(path) {
                    return false;
                }
                parent = path.parent();
            }
            true
        })
        .cloned()
        .collect();

    Ok(RemovalPlan {
        root,
        direct_match_count,
        keep_items,
        delete_items,
        delete_roots,
    })
}

/// 保持パターンをGlobSetに変換し、リテラルのディレクトリ指定を記録します。
fn compile_patterns(root: &Path, patterns: &[String]) -> Result<(GlobSet, Vec<PatternSpec>)> {
    let root_text = normalize_absolute_for_match(&root.to_string_lossy());
    let mut builder = GlobSetBuilder::new();
    let mut specs = Vec::with_capacity(patterns.len());

    for raw in patterns {
        let raw = raw.trim();
        if raw.is_empty() {
            bail!("Pattern must not be empty");
        }
        if raw.split(['/', '\\']).any(|component| component == "..") {
            bail!("Parent directory traversal is not allowed in pattern: {raw}");
        }

        let pattern = normalize_pattern(&root_text, raw)?;
        let matcher = GlobBuilder::new(&pattern)
            .literal_separator(true)
            .case_insensitive(cfg!(windows))
            .build()
            .with_context(|| format!("Invalid glob pattern: {raw}"))?;
        let glob_index = specs.len();
        builder.add(matcher);

        let prefix = if !pattern.contains(['*', '?', '[']) {
            Some(pattern.trim_end_matches('/').to_owned())
        } else {
            None
        };
        specs.push(PatternSpec { glob_index, prefix });
    }

    Ok((builder.build()?, specs))
}

/// 絶対パターンをルート相対へ変換し、ルート外の指定を拒否します。
fn normalize_pattern(root_text: &str, pattern: &str) -> Result<String> {
    if is_drive_relative_path(pattern) {
        bail!("Drive-relative patterns are not supported: {pattern}");
    }

    let normalized = normalize_absolute_for_match(pattern);
    let root_text = normalize_absolute_for_match(root_text);

    if is_absolute_pattern(pattern) {
        let boundary_root = root_text.trim_end_matches('/');
        let root_matches = |candidate: &str| {
            if cfg!(windows) {
                candidate.eq_ignore_ascii_case(boundary_root)
            } else {
                candidate == boundary_root
            }
        };
        let is_within_root = root_matches(&normalized)
            || normalized.get(..boundary_root.len()).is_some_and(|prefix| {
                root_matches(prefix) && normalized[boundary_root.len()..].starts_with('/')
            });
        if !is_within_root {
            bail!("Absolute pattern is outside the processing root: {pattern}");
        }
        let relative = normalized[boundary_root.len()..].trim_start_matches('/');
        Ok(relative.to_owned())
    } else {
        let mut relative = normalized.as_str();
        while let Some(stripped) = relative.strip_prefix("./") {
            relative = stripped;
        }
        let relative = relative.trim_matches('/');
        Ok(relative.to_owned())
    }
}

/// パスをGlob照合用に正規化します。Windowsでは区切りと拡張パス表記も正規化します。
fn normalize_absolute_for_match(path: &str) -> String {
    let normalized = if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    };
    if cfg!(windows) {
        if let Some(unc_path) = normalized.strip_prefix("//?/UNC/") {
            return format!("//{unc_path}");
        }
        if let Some(path) = normalized.strip_prefix("//?/") {
            return path.to_owned();
        }
    }
    normalized
}

/// パスが指定ディレクトリとその配下に含まれるか判定します。
fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let starts_with_prefix = if cfg!(windows) {
        path.get(..prefix.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
    } else {
        path.starts_with(prefix)
    };
    starts_with_prefix
        && (path.len() == prefix.len()
            || path
                .get(prefix.len()..)
                .is_some_and(|suffix| suffix.starts_with('/')))
}

fn is_absolute_pattern(pattern: &str) -> bool {
    Path::new(pattern).is_absolute()
        || (cfg!(windows)
            && (matches!(pattern.as_bytes().first(), Some(b'/' | b'\\'))
                || (pattern.as_bytes().get(1) == Some(&b':')
                    && pattern
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphabetic)
                    && pattern
                        .as_bytes()
                        .get(2)
                        .is_some_and(|separator| matches!(separator, b'/' | b'\\')))))
}

fn is_drive_relative_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    cfg!(windows)
        && bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && !bytes
            .get(2)
            .is_some_and(|separator| matches!(separator, b'/' | b'\\'))
}

fn is_filesystem_root(path: &Path) -> bool {
    path.components()
        .all(|component| !matches!(component, Component::Normal(_)))
}

/// 相対パスをプラットフォームに依存しない `/` 区切りの表示形式にします。
fn display_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::{ItemType, build_plan, is_filesystem_root};
    #[cfg(windows)]
    use super::{is_drive_relative_path, normalize_pattern};
    use assert_fs::prelude::*;
    use std::path::Path;

    #[test]
    // UnixとWindowsのファイルシステムルートを正しく判定することを確認する。
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
    // Windowsのドライブ相対パスをルートと保持パターンの両方で拒否することを確認する。
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
    // ファイルシステムルートを走査開始前に拒否することを確認する。
    fn rejects_filesystem_root_before_walking_it() {
        let error = build_plan(Path::new("/"), &["*".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("Filesystem roots"));
    }

    #[test]
    // パターン未指定、空パターン、不正なglobを拒否することを確認する。
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
    // ディレクトリ以外を処理ルートとして指定すると拒否することを確認する。
    fn rejects_non_directory_root() {
        let temp = assert_fs::TempDir::new().unwrap();
        let root_file = temp.child("root.txt");
        root_file.write_str("not a directory").unwrap();

        let error = build_plan(root_file.path(), &["*".to_owned()]).unwrap_err();

        assert!(error.to_string().contains("Root path is not a directory"));
    }

    #[test]
    // パターン一致項目と祖先を保持し、それ以外を削除計画に含めることを確認する。
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
    // リテラルのディレクトリ指定が配下全体を保持することを確認する。
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
    // * は直下、** は複数階層のパスに一致することを確認する。
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
    // ? と文字クラスがそれぞれ1文字に一致することを確認する。
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
    // dir/* が直下の項目だけに一致し、子孫までは保持しないことを確認する。
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
    // 親ディレクトリへの移動を含む保持パターンを拒否することを確認する。
    fn rejects_parent_traversal() {
        let temp = assert_fs::TempDir::new().unwrap();
        let error = build_plan(temp.path(), &["../outside".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("Parent directory traversal"));
    }

    #[test]
    // ルート外を指す絶対保持パターンを拒否することを確認する。
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
    // ルート内の絶対保持パターンで対象項目を保持できることを確認する。
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
    // Globの大文字・小文字の照合がプラットフォームごとの仕様に従うことを確認する。
    fn glob_case_sensitivity_matches_platform() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("Keep.txt").write_str("keep").unwrap();

        let plan = build_plan(temp.path(), &["keep.txt".to_owned()]).unwrap();

        assert_eq!(plan.delete_items.is_empty(), cfg!(windows));
    }

    #[cfg(windows)]
    #[test]
    // Windowsのルート相対パターンが選択ルート外なら拒否することを確認する。
    fn rejects_windows_root_relative_pattern_outside_current_root() {
        let temp = assert_fs::TempDir::new().unwrap();

        let error = build_plan(temp.path(), &["/outside/*".to_owned()]).unwrap_err();

        assert!(error.to_string().contains("outside the processing root"));
    }
}
